//! Game-side connection to a `rally_server`. Non-blocking: call `poll` once per frame.
use crate::conn::Conn;
use crate::protocol::*;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Connecting,
    /// Connected, waiting for Welcome
    Joining,
    Connected,
    /// Ended, with a reason for the UI
    Disconnected(String),
}

/// Race-relevant events for the game (lobby state is kept in the client itself)
#[derive(Clone, Debug, PartialEq)]
pub enum NetEvent {
    Start(RaceStart),
    Go { race_id: u32, countdown: f32 },
    State { id: PlayerId, snap: CarSnapshot },
    Finished { id: PlayerId, time: f32 },
    Left { id: PlayerId },
    RaceOver { race_id: u32, results: Vec<(PlayerId, Option<f32>)> },
}

pub struct NetClient {
    pub status: Status,
    pub address: String,
    pub my_id: PlayerId,
    pub server_name: String,
    pub players: Vec<PlayerInfo>,
    pub leader: PlayerId,
    pub settings: Option<RaceSettings>,
    pub race_running: bool,
    /// Smoothed round trip time (ms)
    pub rtt_ms: f32,
    pending: Option<mpsc::Receiver<Result<TcpStream, String>>>,
    conn: Option<Conn>,
    hello: ClientMsg,
    epoch: Instant,
    last_ping: f64,
    last_rx: Instant,
    /// UDP channel for snapshots (None until Welcome, or if the server has no UDP)
    udp: Option<UdpSocket>,
    udp_token: u64,
    /// The server acknowledged our UDP hello: snapshots go over UDP
    pub udp_ok: bool,
    /// Set false to force the TCP fallback (tests, or a "TCP only" option)
    pub udp_allowed: bool,
    last_udp_hello: f64,
    last_udp_ack: f64,
    open_udp_pending: bool,
}

/// "host", "host:port", "1.2.3.4" ... -> socket addresses (default port added)
pub fn resolve(address: &str) -> Result<Vec<SocketAddr>, String> {
    let a = address.trim();
    if a.is_empty() {
        return Err("Enter the server address".into());
    }
    let with_port = if a.parse::<SocketAddr>().is_ok() || (a.contains(':') && !a.contains("::") && a.rsplit(':').next().is_some_and(|p| p.parse::<u16>().is_ok())) {
        a.to_string()
    } else if a.contains("::") && !a.starts_with('[') {
        format!("[{a}]:{DEFAULT_PORT}")
    } else {
        format!("{a}:{DEFAULT_PORT}")
    };
    with_port.to_socket_addrs().map(|i| i.collect()).map_err(|e| format!("Can't find {a}: {e}"))
}

impl NetClient {
    /// Start connecting in the background (never blocks the frame)
    pub fn connect(address: &str, name: &str, car: u8, paint: u8) -> NetClient {
        let (tx, rx) = mpsc::channel();
        let addr = address.to_string();
        std::thread::spawn(move || {
            let r = resolve(&addr).and_then(|addrs| {
                let mut last = String::from("no address");
                for a in addrs {
                    match TcpStream::connect_timeout(&a, Duration::from_secs(5)) {
                        Ok(s) => return Ok(s),
                        Err(e) => last = friendly_error(&e),
                    }
                }
                Err(last)
            });
            let _ = tx.send(r);
        });
        let now = Instant::now();
        NetClient {
            status: Status::Connecting,
            address: address.to_string(),
            my_id: 0,
            server_name: String::new(),
            players: Vec::new(),
            leader: 0,
            settings: None,
            race_running: false,
            rtt_ms: 0.0,
            pending: Some(rx),
            conn: None,
            hello: ClientMsg::Hello { version: PROTOCOL_VERSION, name: name.to_string(), car, paint },
            epoch: now,
            last_ping: -10.0,
            last_rx: now,
            udp: None,
            udp_token: 0,
            udp_ok: false,
            udp_allowed: true,
            last_udp_hello: -10.0,
            last_udp_ack: 0.0,
            open_udp_pending: false,
        }
    }

    pub fn is_leader(&self) -> bool {
        self.status == Status::Connected && self.leader == self.my_id
    }

    pub fn me(&self) -> Option<&PlayerInfo> {
        self.players.iter().find(|p| p.id == self.my_id)
    }

    pub fn player(&self, id: PlayerId) -> Option<&PlayerInfo> {
        self.players.iter().find(|p| p.id == id)
    }

    /// Send a message. Car snapshots go over UDP once it works, everything else over TCP.
    pub fn send(&mut self, msg: &ClientMsg) {
        if let (ClientMsg::State { race_id, snap }, true, Some(u)) = (msg, self.udp_ok, &self.udp) {
            let d = UdpMsg::State { id: self.my_id, token: self.udp_token, race_id: *race_id, snap: *snap };
            if let Ok(bytes) = bincode::serialize(&d) {
                if u.send(&bytes).is_ok() {
                    return;
                }
            }
        }
        if let Some(c) = self.conn.as_mut() {
            c.send(msg);
        }
    }

    /// Open the UDP channel toward the server's TCP address (same port)
    fn open_udp(&mut self) {
        let Some(peer) = self.conn.as_ref().and_then(|c| c.peer) else { return };
        let bind = if peer.is_ipv6() { "[::]:0" } else { "0.0.0.0:0" };
        self.udp = UdpSocket::bind(bind).and_then(|u| u.connect(peer).map(|_| u)).and_then(|u| u.set_nonblocking(true).map(|_| u)).ok();
    }

    fn poll_udp(&mut self, now: f64, events: &mut Vec<NetEvent>) {
        let Some(u) = &self.udp else { return };
        let mut buf = [0u8; MAX_DATAGRAM];
        while let Ok(n) = u.recv(&mut buf) {
            match bincode::deserialize::<UdpMsg>(&buf[..n]) {
                Ok(UdpMsg::Ack) => {
                    self.last_udp_ack = now;
                    self.udp_ok = self.udp_allowed;
                }
                Ok(UdpMsg::Relay { id, snap }) => {
                    self.last_udp_ack = now;
                    events.push(NetEvent::State { id, snap });
                }
                _ => {}
            }
        }
        // hello: quickly until acknowledged, then every few seconds to keep NAT mappings open
        let every = if self.udp_ok { 3.0 } else { 0.5 };
        if self.udp_allowed && now - self.last_udp_hello > every {
            self.last_udp_hello = now;
            if let Ok(b) = bincode::serialize(&UdpMsg::Hello { id: self.my_id, token: self.udp_token }) {
                let _ = u.send(&b);
            }
        }
        // no answer for a while: back to TCP until UDP answers again
        if self.udp_ok && now - self.last_udp_ack > 10.0 {
            self.udp_ok = false;
        }
        if !self.udp_allowed {
            self.udp_ok = false;
        }
    }

    pub fn disconnect(&mut self) {
        self.udp = None;
        self.udp_ok = false;
        if let Some(mut c) = self.conn.take() {
            c.flush();
            c.close();
        }
        self.status = Status::Disconnected("Left the server".into());
    }

    /// Pump the connection; returns race events in arrival order
    pub fn poll(&mut self) -> Vec<NetEvent> {
        let mut events = Vec::new();
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(Ok(stream)) => {
                    self.pending = None;
                    match Conn::new(stream) {
                        Ok(mut c) => {
                            c.send(&self.hello);
                            self.conn = Some(c);
                            self.status = Status::Joining;
                            self.last_rx = Instant::now();
                        }
                        Err(e) => self.status = Status::Disconnected(friendly_error(&e)),
                    }
                }
                Ok(Err(e)) => {
                    self.pending = None;
                    self.status = Status::Disconnected(e);
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.status = Status::Disconnected("Connection failed".into());
                }
            }
        }
        let Some(conn) = self.conn.as_mut() else { return events };
        let now = self.epoch.elapsed().as_secs_f64();
        let msgs: Vec<ServerMsg> = conn.recv_all();
        if !msgs.is_empty() {
            self.last_rx = Instant::now();
        }
        for m in msgs {
            match m {
                ServerMsg::Welcome { id, server_name, udp_token } => {
                    self.my_id = id;
                    self.server_name = server_name;
                    self.status = Status::Connected;
                    self.udp_token = udp_token;
                    if udp_token != 0 {
                        self.open_udp_pending = true;
                    }
                }
                ServerMsg::Refused { reason } => self.status = Status::Disconnected(reason),
                ServerMsg::Lobby { players, leader, settings, race_running } => {
                    self.players = players;
                    self.leader = leader;
                    self.settings = settings;
                    self.race_running = race_running;
                }
                ServerMsg::Pong { t } => {
                    let rtt = ((now - t) * 1000.0) as f32;
                    self.rtt_ms = if self.rtt_ms <= 0.0 { rtt } else { self.rtt_ms + (rtt - self.rtt_ms) * 0.3 };
                }
                ServerMsg::Start(s) => events.push(NetEvent::Start(s)),
                ServerMsg::Go { race_id, countdown } => events.push(NetEvent::Go { race_id, countdown }),
                ServerMsg::State { id, snap } => events.push(NetEvent::State { id, snap }),
                ServerMsg::Finished { id, time } => events.push(NetEvent::Finished { id, time }),
                ServerMsg::Left { id } => events.push(NetEvent::Left { id }),
                ServerMsg::RaceOver { race_id, results } => events.push(NetEvent::RaceOver { race_id, results }),
            }
        }
        if std::mem::take(&mut self.open_udp_pending) {
            self.open_udp();
        }
        self.poll_udp(now, &mut events);
        let Some(conn) = self.conn.as_mut() else { return events };
        if now - self.last_ping > 1.0 {
            self.last_ping = now;
            conn.send(&ClientMsg::Ping { t: now, rtt_ms: self.rtt_ms.round().clamp(0.0, 9999.0) as u16 });
        }
        conn.flush();
        let silent = self.last_rx.elapsed() > Duration::from_secs(10);
        if conn.is_closed() || silent {
            if !matches!(self.status, Status::Disconnected(_)) {
                self.status = Status::Disconnected(if silent { "Lost connection to the server".into() } else { "The server closed the connection".into() });
            }
            self.conn = None;
        }
        events
    }
}

fn friendly_error(e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    match e.kind() {
        ConnectionRefused => "Nobody is hosting there (connection refused) - is rally_server running?".into(),
        TimedOut | WouldBlock => "No answer - check the address, port forwarding or Tailscale".into(),
        _ => format!("Can't connect: {e}"),
    }
}
