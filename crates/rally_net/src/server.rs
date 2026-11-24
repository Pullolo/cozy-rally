//! Self-hostable race server: lobby, leader, synchronised start, snapshot relay, results.
//! Single-threaded and non-blocking; call `poll` in a loop (the `rally_server` binary does).
use crate::conn::Conn;
use crate::protocol::*;
use std::io;
use std::net::{SocketAddr, TcpListener};
use std::time::{Duration, Instant};

/// Wait this long for slow loaders before starting anyway
const LOAD_TIMEOUT: Duration = Duration::from_secs(25);
/// After the first finisher the others get this long
const FINISH_GRACE: Duration = Duration::from_secs(60);
/// Drop clients that sent nothing for this long
const IDLE_TIMEOUT: Duration = Duration::from_secs(15);
/// Hard cap on a race (someone parked forever)
const RACE_MAX: Duration = Duration::from_secs(60 * 45);
/// Number of paints in the game (rally_game::models::PAINTS)
const PAINT_COUNT: u8 = 9;

struct Client {
    id: PlayerId,
    conn: Conn,
    /// None until Hello
    info: Option<PlayerInfo>,
    last_seen: Instant,
}

struct RaceRun {
    start: RaceStart,
    /// players (ids) still in the race
    racers: Vec<PlayerId>,
    loaded: Vec<PlayerId>,
    started_at: Instant,
    go_sent: bool,
    finished: Vec<(PlayerId, f32)>,
    first_finish: Option<Instant>,
}

pub struct Server {
    listener: TcpListener,
    pub name: String,
    clients: Vec<Client>,
    next_id: PlayerId,
    leader: Option<PlayerId>,
    settings: Option<RaceSettings>,
    race: Option<RaceRun>,
    next_race: u32,
    lobby_dirty: bool,
}

impl Server {
    pub fn bind(addr: &str, name: &str) -> io::Result<Server> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        Ok(Server { listener, name: name.to_string(), clients: Vec::new(), next_id: 1, leader: None, settings: None, race: None, next_race: 1, lobby_dirty: false })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn player_count(&self) -> usize {
        self.clients.iter().filter(|c| c.info.is_some()).count()
    }

    /// Accept, read, process and flush. Returns log lines for the console.
    pub fn poll(&mut self) -> Vec<String> {
        let now = Instant::now();
        let mut log = Vec::new();
        while let Ok((stream, addr)) = self.listener.accept() {
            if let Ok(conn) = Conn::new(stream) {
                let id = self.alloc_id();
                self.clients.push(Client { id, conn, info: None, last_seen: now });
                log.push(format!("connection from {addr}"));
            }
        }
        for ci in 0..self.clients.len() {
            let msgs: Vec<ClientMsg> = self.clients[ci].conn.recv_all();
            if !msgs.is_empty() {
                self.clients[ci].last_seen = now;
            }
            for m in msgs {
                self.handle(ci, m, now, &mut log);
            }
        }
        // drop dead / idle clients
        let mut gone = Vec::new();
        self.clients.retain(|c| {
            let dead = c.conn.is_closed() || now.duration_since(c.last_seen) > IDLE_TIMEOUT;
            if dead {
                if let Some(i) = &c.info {
                    gone.push((c.id, i.name.clone()));
                }
            }
            !dead
        });
        for (id, name) in gone {
            log.push(format!("{name} left"));
            self.remove_from_race(id);
            self.broadcast(&ServerMsg::Left { id }, None);
            if self.leader == Some(id) {
                self.leader = self.clients.iter().find(|c| c.info.is_some()).map(|c| c.id);
            }
            self.lobby_dirty = true;
        }
        self.update_race(now, &mut log);
        if self.lobby_dirty {
            self.lobby_dirty = false;
            self.send_lobby();
        }
        for c in self.clients.iter_mut() {
            c.conn.flush();
        }
        log
    }

    fn alloc_id(&mut self) -> PlayerId {
        loop {
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1).max(1);
            if !self.clients.iter().any(|c| c.id == id) {
                return id;
            }
        }
    }

    fn handle(&mut self, ci: usize, m: ClientMsg, now: Instant, log: &mut Vec<String>) {
        let id = self.clients[ci].id;
        let joined = self.clients[ci].info.is_some();
        match m {
            ClientMsg::Hello { version, name, car, paint } => {
                if joined {
                    return;
                }
                let refuse = if version != PROTOCOL_VERSION {
                    Some(format!("Version mismatch (server {PROTOCOL_VERSION}, you {version}) - update the game"))
                } else if self.player_count() >= MAX_PLAYERS {
                    Some(format!("Server is full ({MAX_PLAYERS} players)"))
                } else {
                    None
                };
                if let Some(reason) = refuse {
                    let c = &mut self.clients[ci];
                    c.conn.send(&ServerMsg::Refused { reason });
                    c.conn.flush();
                    c.conn.close();
                    return;
                }
                let name = clean_name(&name, id);
                log.push(format!("{name} joined (id {id})"));
                self.clients[ci].info = Some(PlayerInfo { id, name, car, paint, ready: false, ping_ms: 0, racing: false });
                let welcome = ServerMsg::Welcome { id, server_name: self.name.clone() };
                self.clients[ci].conn.send(&welcome);
                if self.leader.is_none() {
                    self.leader = Some(id);
                }
                self.lobby_dirty = true;
            }
            _ if !joined => {}
            ClientMsg::SetCar { car, paint } => {
                if let Some(i) = self.clients[ci].info.as_mut() {
                    i.car = car;
                    i.paint = paint;
                }
                self.lobby_dirty = true;
            }
            ClientMsg::SetReady(r) => {
                if let Some(i) = self.clients[ci].info.as_mut() {
                    i.ready = r;
                }
                self.lobby_dirty = true;
            }
            ClientMsg::SetSettings(s) => {
                if self.leader == Some(id) && self.race.is_none() {
                    self.settings = Some(s);
                    self.lobby_dirty = true;
                }
            }
            ClientMsg::StartRace => {
                if self.leader == Some(id) && self.race.is_none() {
                    self.start_race(now, log);
                }
            }
            ClientMsg::Loaded { race_id } => {
                if let Some(r) = self.race.as_mut().filter(|r| r.start.race_id == race_id) {
                    if r.racers.contains(&id) && !r.loaded.contains(&id) {
                        r.loaded.push(id);
                    }
                }
            }
            ClientMsg::State { race_id, snap } => {
                let racing = self.race.as_ref().is_some_and(|r| r.start.race_id == race_id && r.racers.contains(&id));
                if racing {
                    let ids: Vec<PlayerId> = self.race.as_ref().map(|r| r.racers.clone()).unwrap_or_default();
                    let msg = ServerMsg::State { id, snap };
                    for c in self.clients.iter_mut().filter(|c| c.id != id && ids.contains(&c.id)) {
                        c.conn.send(&msg);
                    }
                }
            }
            ClientMsg::Finished { race_id, time } => {
                let mut announce = false;
                if let Some(r) = self.race.as_mut().filter(|r| r.start.race_id == race_id && r.go_sent) {
                    if r.racers.contains(&id) && !r.finished.iter().any(|(p, _)| *p == id) && time.is_finite() && time > 0.0 {
                        r.finished.push((id, time));
                        r.first_finish.get_or_insert(now);
                        announce = true;
                    }
                }
                if announce {
                    log.push(format!("player {id} finished in {time:.2}s"));
                    self.broadcast(&ServerMsg::Finished { id, time }, None);
                }
            }
            ClientMsg::LeaveRace { race_id } => {
                if self.race.as_ref().is_some_and(|r| r.start.race_id == race_id) {
                    self.remove_from_race(id);
                    self.broadcast(&ServerMsg::Left { id }, None);
                    self.lobby_dirty = true;
                }
            }
            ClientMsg::Ping { t, rtt_ms } => {
                self.clients[ci].conn.send(&ServerMsg::Pong { t });
                if let Some(i) = self.clients[ci].info.as_mut() {
                    if i.ping_ms.abs_diff(rtt_ms) > 4 {
                        self.lobby_dirty = true;
                    }
                    i.ping_ms = rtt_ms;
                }
            }
        }
    }

    fn start_race(&mut self, now: Instant, log: &mut Vec<String>) {
        let Some(settings) = self.settings.clone() else { return };
        let mut grid: Vec<GridEntry> = self.clients.iter().filter_map(|c| c.info.as_ref()).map(|i| GridEntry { id: i.id, name: i.name.clone(), car: i.car, paint: i.paint }).collect();
        // friends who picked the same paint get told apart: later joiners shift to a free colour
        for k in 1..grid.len() {
            let mut p = grid[k].paint;
            let mut guard = 0;
            while grid[..k].iter().any(|g| g.paint == p) && guard < PAINT_COUNT {
                p = (p + 1) % PAINT_COUNT;
                guard += 1;
            }
            grid[k].paint = p;
        }
        if grid.is_empty() {
            return;
        }
        let race_id = self.next_race;
        self.next_race += 1;
        let start = RaceStart { race_id, settings, grid };
        log.push(format!("race {race_id} starting with {} players", start.grid.len()));
        self.broadcast(&ServerMsg::Start(start.clone()), None);
        let racers = start.grid.iter().map(|g| g.id).collect();
        for c in self.clients.iter_mut() {
            if let Some(i) = c.info.as_mut() {
                i.ready = false;
                i.racing = true;
            }
        }
        self.race = Some(RaceRun { start, racers, loaded: Vec::new(), started_at: now, go_sent: false, finished: Vec::new(), first_finish: None });
        self.lobby_dirty = true;
    }

    fn remove_from_race(&mut self, id: PlayerId) {
        if let Some(r) = self.race.as_mut() {
            r.racers.retain(|p| *p != id);
            r.loaded.retain(|p| *p != id);
        }
        if let Some(i) = self.clients.iter_mut().find(|c| c.id == id).and_then(|c| c.info.as_mut()) {
            i.racing = false;
        }
    }

    fn update_race(&mut self, now: Instant, log: &mut Vec<String>) {
        let Some(r) = self.race.as_mut() else { return };
        if !r.go_sent && (r.racers.iter().all(|p| r.loaded.contains(p)) || now.duration_since(r.started_at) > LOAD_TIMEOUT) {
            r.go_sent = true;
            let msg = ServerMsg::Go { race_id: r.start.race_id, countdown: COUNTDOWN };
            log.push(format!("race {} go", r.start.race_id));
            self.broadcast(&msg, None);
            return;
        }
        let r = self.race.as_ref().unwrap();
        let all_done = r.racers.iter().all(|p| r.finished.iter().any(|(f, _)| f == p));
        let grace_over = r.first_finish.is_some_and(|t| now.duration_since(t) > FINISH_GRACE);
        let too_long = now.duration_since(r.started_at) > RACE_MAX;
        if (r.go_sent && all_done) || grace_over || too_long || r.racers.is_empty() {
            let mut finished = r.finished.clone();
            finished.sort_by(|a, b| a.1.total_cmp(&b.1));
            let mut results: Vec<(PlayerId, Option<f32>)> = finished.iter().map(|(p, t)| (*p, Some(*t))).collect();
            for g in &r.start.grid {
                if !results.iter().any(|(p, _)| *p == g.id) {
                    results.push((g.id, None));
                }
            }
            let race_id = r.start.race_id;
            log.push(format!("race {race_id} over"));
            self.race = None;
            for c in self.clients.iter_mut() {
                if let Some(i) = c.info.as_mut() {
                    i.racing = false;
                }
            }
            self.broadcast(&ServerMsg::RaceOver { race_id, results }, None);
            self.lobby_dirty = true;
        }
    }

    fn send_lobby(&mut self) {
        let players: Vec<PlayerInfo> = self.clients.iter().filter_map(|c| c.info.clone()).collect();
        let msg = ServerMsg::Lobby { players, leader: self.leader.unwrap_or(0), settings: self.settings.clone(), race_running: self.race.is_some() };
        self.broadcast(&msg, None);
    }

    fn broadcast(&mut self, msg: &ServerMsg, except: Option<PlayerId>) {
        for c in self.clients.iter_mut().filter(|c| c.info.is_some() && Some(c.id) != except) {
            c.conn.send(msg);
        }
    }
}

fn clean_name(name: &str, id: PlayerId) -> String {
    let n: String = name.chars().filter(|c| !c.is_control()).take(16).collect();
    let n = n.trim().to_string();
    if n.is_empty() { format!("Racer {id}") } else { n }
}
