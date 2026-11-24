//! Online races with friends via a self-hosted `rally_server` (see crates/rally_net).
//!
//! Flow: Title -> "Online" -> connect screen (address + name) -> lobby (players, cars, ready,
//! leader picks the race in the normal Setup screen) -> leader starts -> everyone loads the
//! same track and waits at the line -> server says Go -> race (remote cars via rally_net::sync)
//! -> results (official times from the server) -> back to the lobby.
use crate::game::{PlayerSlot, RaceSession, SessionConfig};
use crate::input::Scheme;
use crate::models::PAINTS;
use crate::ui::*;
use macroquad::prelude::*;
use rally_core::world::Controller;
use rally_net::sync::{snapshot_of, RemoteCar};
use rally_net::*;

/// Grid slot data for an online session (one per car, in World order)
#[derive(Clone, Debug)]
pub struct OnlineGrid {
    pub entries: Vec<GridEntry>,
    pub my_id: PlayerId,
    pub contact: bool,
}

pub struct OnlineRace {
    pub race_id: u32,
    pub my_id: PlayerId,
    /// player id per car slot
    pub slots: Vec<PlayerId>,
    pub remotes: Vec<Option<RemoteCar>>,
    pub go: bool,
    sent_finish: bool,
    send_acc: f32,
    /// Official results from the server (finish order, None = DNF)
    pub official: Option<Vec<(PlayerId, Option<f32>)>>,
}

pub struct Online {
    pub client: Option<NetClient>,
    pub address: String,
    pub name: String,
    /// Text field being edited: 0 address, 1 name
    editing: Option<u8>,
    pub pending_start: Option<RaceStart>,
    pub race: Option<OnlineRace>,
    /// Last disconnect reason (shown on the connect screen)
    pub message: Option<String>,
    /// Leader: the Setup screen is picking the online race
    pub setup_for_online: bool,
    pub contact: bool,
    inbox: Vec<NetEvent>,
    sent_car: Option<(u8, u8)>,
}

impl Online {
    pub fn new() -> Online {
        let (address, name) = crate::settings::load_online();
        Online {
            client: None,
            address,
            name: if name.is_empty() { "Racer".into() } else { name },
            editing: None,
            pending_start: None,
            race: None,
            message: None,
            setup_for_online: false,
            contact: false,
            inbox: Vec::new(),
            sent_car: None,
        }
    }

    pub fn connected(&self) -> bool {
        self.client.as_ref().is_some_and(|c| c.status == Status::Connected)
    }

    pub fn in_race(&self) -> bool {
        self.race.is_some()
    }

    pub fn send(&mut self, m: &ClientMsg) {
        if let Some(c) = self.client.as_mut() {
            c.send(m);
        }
    }

    /// Pump the network once per frame (on every screen). Returns true when a race starts.
    pub fn poll(&mut self) -> bool {
        let Some(c) = self.client.as_mut() else { return false };
        let mut start = false;
        for e in c.poll() {
            match e {
                NetEvent::Start(s) => {
                    if s.grid.iter().any(|g| g.id == c.my_id) {
                        self.pending_start = Some(s);
                        self.race = None;
                        self.inbox.clear();
                        start = true;
                    }
                }
                other => self.inbox.push(other),
            }
        }
        if let Status::Disconnected(reason) = &c.status {
            self.message = Some(reason.clone());
            self.client = None;
            self.race = None;
            self.pending_start = None;
            self.sent_car = None;
        }
        start
    }

    pub fn disconnect(&mut self) {
        if let Some(c) = self.client.as_mut() {
            c.disconnect();
        }
        self.client = None;
        self.race = None;
        self.pending_start = None;
        self.sent_car = None;
        self.message = None;
    }

    /// Keep the server's view of my car in sync with the setup screen choice
    pub fn sync_car(&mut self, car: usize, paint: usize) {
        let want = (car as u8, paint as u8);
        if self.connected() && self.sent_car != Some(want) {
            self.sent_car = Some(want);
            self.send(&ClientMsg::SetCar { car: want.0, paint: want.1 });
        }
    }

    /// Session config for the race the server just started (consumes it)
    pub fn take_config(&mut self, assist_level: f32) -> Option<SessionConfig> {
        let s = self.pending_start.take()?;
        let my_id = self.client.as_ref()?.my_id;
        let me = s.grid.iter().find(|g| g.id == my_id)?;
        let player = PlayerSlot { car: me.car as usize, paint: me.paint as usize, scheme: Scheme::Both, name: me.name.clone() };
        let slots: Vec<PlayerId> = s.grid.iter().map(|g| g.id).collect();
        self.race = Some(OnlineRace { race_id: s.race_id, my_id, remotes: slots.iter().map(|&id| if id == my_id { None } else { Some(RemoteCar::default()) }).collect(), slots, go: false, sent_finish: false, send_acc: 0.0, official: None });
        Some(SessionConfig {
            track: s.settings.track.clone(),
            laps: s.settings.laps,
            players: vec![player],
            ai_count: 0,
            ai_skill: 0.7,
            assist_level,
            mode: crate::gameplay::GameMode::Race,
            online: Some(OnlineGrid { entries: s.grid.clone(), my_id, contact: s.settings.contact }),
        })
    }

    /// The session is built: wait at the line until everyone has loaded
    pub fn session_ready(&mut self, sess: &mut RaceSession) {
        if let Some(r) = &self.race {
            sess.hold = true;
            let id = r.race_id;
            self.send(&ClientMsg::Loaded { race_id: id });
        }
    }

    /// Quit the current race (back to the lobby)
    pub fn leave_race(&mut self) {
        if let Some(r) = self.race.take() {
            self.send(&ClientMsg::LeaveRace { race_id: r.race_id });
        }
    }

    /// Before `RaceSession::update`: apply network events and steer remote cars
    pub fn pre_update(&mut self, sess: &mut RaceSession, dt: f32) {
        let Some(r) = self.race.as_mut() else { return };
        let rtt = self.client.as_ref().map_or(100.0, |c| c.rtt_ms);
        for e in self.inbox.drain(..) {
            match e {
                NetEvent::Go { race_id, .. } if race_id == r.race_id => {
                    r.go = true;
                    sess.hold = false;
                }
                NetEvent::State { id, snap } => {
                    if let Some(slot) = r.slots.iter().position(|&p| p == id) {
                        if let Some(rc) = r.remotes[slot].as_mut() {
                            rc.receive(snap, sess.time);
                        }
                    }
                }
                NetEvent::Finished { id, time } => {
                    if let Some(slot) = r.slots.iter().position(|&p| p == id) {
                        let race = &mut sess.world.race;
                        if race.racers[slot].finished {
                            race.racers[slot].finish_time = Some(time); // official time wins
                        } else {
                            race.force_finish(slot, time);
                        }
                    }
                }
                NetEvent::Left { id } => {
                    if let Some(slot) = r.slots.iter().position(|&p| p == id && p != r.my_id) {
                        // a friend dropped: an AI driver takes their car home
                        if r.remotes[slot].take().is_some() {
                            sess.world.controllers[slot] = Controller::Ai;
                            sess.ais[slot] = Some(rally_core::ai::AiDriver::new(0.6, 0.0));
                            sess.names[slot] = format!("{} (left)", sess.names[slot]);
                        }
                    }
                }
                NetEvent::RaceOver { race_id, results } if race_id == r.race_id => {
                    for (id, t) in &results {
                        if let (Some(slot), Some(t)) = (r.slots.iter().position(|p| p == id), t) {
                            sess.world.race.racers[slot].finish_time = Some(*t);
                        }
                    }
                    r.official = Some(results);
                }
                _ => {}
            }
        }
        let latency = rtt * 0.5 / 1000.0 + 0.5 / SNAPSHOT_HZ;
        for (slot, rc) in r.remotes.iter_mut().enumerate() {
            if let Some(rc) = rc {
                if let Some(inp) = rc.input() {
                    sess.world.set_input(slot, inp);
                }
                if r.go {
                    rc.correct(&mut sess.world.cars[slot], sess.time, latency, dt);
                }
            }
        }
    }

    /// After `RaceSession::update`: stream my car, report my finish
    pub fn post_update(&mut self, sess: &RaceSession, dt: f32) {
        let Some(r) = self.race.as_mut() else { return };
        if !r.go {
            return;
        }
        let Some(me) = r.slots.iter().position(|&p| p == r.my_id) else { return };
        let mut out = Vec::new();
        r.send_acc += dt;
        if r.send_acc >= 1.0 / SNAPSHOT_HZ {
            r.send_acc = r.send_acc.rem_euclid(1.0 / SNAPSHOT_HZ);
            out.push(ClientMsg::State { race_id: r.race_id, snap: snapshot_of(&sess.world.cars[me], sess.time) });
        }
        let racer = &sess.world.race.racers[me];
        if racer.finished && !r.sent_finish {
            if let Some(t) = racer.finish_time {
                r.sent_finish = true;
                out.push(ClientMsg::Finished { race_id: r.race_id, time: t });
            }
        }
        for m in out {
            self.send(&m);
        }
    }

    /// In-race status line: waiting at the line, or a friend whose car went quiet
    pub fn status_line(&self, sess: &RaceSession) -> Option<String> {
        let r = self.race.as_ref()?;
        if !r.go {
            return Some("Waiting for everyone to reach the line...".into());
        }
        let quiet: Vec<&str> = r.remotes.iter().enumerate().filter(|(_, rc)| rc.as_ref().is_some_and(|rc| rc.count > 0 && rc.silence(sess.time) > 1.5)).map(|(i, _)| sess.names[i].as_str()).collect();
        if quiet.is_empty() { None } else { Some(format!("Connection trouble: {}", quiet.join(", "))) }
    }
}

// ----------------------------------------------------------------------------- UI

/// What the lobby screen asks the app to do
pub enum LobbyAction {
    None,
    Back,
    /// Leader: open the race Setup screen to choose the online race
    PickRace,
}

fn car_name(car: u8) -> &'static str {
    rally_core::vehicle::car_specs().get(car as usize).map_or("?", |c| c.name)
}

pub fn race_summary(s: &RaceSettings) -> String {
    let t = &s.track;
    let kind = if t.discipline == rally_core::track::Discipline::Circuit { "Circuit" } else { "Rally" };
    let biome = t.biome.map_or("Random scenery".to_string(), |b| format!("{b:?}"));
    format!("{kind}  -  {biome}  -  {} lap{}  -  seed {}  -  {}", s.laps, if s.laps == 1 { "" } else { "s" }, t.seed, if s.contact { "contact on" } else { "ghost cars" })
}

/// Single-line text box. Click (or Enter on it) to type; Enter / Esc / click outside to finish.
fn text_field(focus: &mut Focus, editing: &mut Option<u8>, which: u8, label: &str, value: &mut String, max: usize, x: f32, y: f32, w: f32, h: f32) {
    let s = scale();
    let id_focused = focus.peek_next() == focus.index;
    let active = *editing == Some(which);
    let (_, clicked) = focus_item(focus, x, y, w, h);
    let r = Rect::new(x, y, w, h);
    let clicked_outside = is_mouse_button_pressed(MouseButton::Left) && !r.contains(mouse_position().into());
    if active {
        while let Some(ch) = get_char_pressed() {
            if !ch.is_control() && value.chars().count() < max {
                value.push(ch);
            }
        }
        if is_key_pressed(KeyCode::Backspace) {
            value.pop();
        }
        if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter) || is_key_pressed(KeyCode::Escape) || clicked_outside {
            *editing = None;
            set_typing(false);
        }
    } else if clicked {
        *editing = Some(which);
        set_typing(true);
        while get_char_pressed().is_some() {}
    }
    let bg = if active { Color::new(1.0, 1.0, 1.0, 0.95) } else if id_focused { mix(CREAM, SKY, 0.35) } else { CREAM };
    rounded_rect(x, y, w, h, 12.0 * s, bg);
    rounded_rect_outline(x, y, w, h, 12.0 * s, 2.0 * s, if active { SKY } else { with_alpha(INK_SOFT, 0.35) });
    text(label, x + 16.0 * s, mid_baseline(y + h * 0.5, 18.0 * s), 18.0 * s, INK_SOFT);
    let vx = x + 150.0 * s;
    let shown = if value.is_empty() && !active { "click to type".to_string() } else { value.clone() };
    text(&shown, vx, mid_baseline(y + h * 0.5, 22.0 * s), 22.0 * s, if value.is_empty() { INK_SOFT } else { INK });
    if active && (get_time() * 2.0) as i64 % 2 == 0 {
        let cw = text_size(value, 22.0 * s).x;
        draw_rectangle(vx + cw + 2.0 * s, y + h * 0.22, 2.0 * s, h * 0.56, INK);
    }
}

impl Online {
    /// The Online screen: connect form, or the lobby once connected
    pub fn screen(&mut self, focus: &mut Focus, car: &mut usize, paint: &mut usize, allowed_cars: &[usize]) -> LobbyAction {
        let s = scale();
        let sw = screen_width();
        let sh = screen_height();
        let w = (760.0 * s).min(sw - 32.0 * s);
        let h = (600.0 * s).min(sh - 32.0 * s);
        let x = (sw - w) * 0.5;
        let y = (sh - h) * 0.5;
        panel(x, y, w, h);
        let pad = 28.0 * s;
        let iw = w - pad * 2.0;
        let ix = x + pad;
        let rh = 50.0 * s;
        match self.client.as_ref().map(|c| c.status.clone()) {
            None => self.connect_form(focus, ix, y, iw, h, rh, car, paint, allowed_cars),
            Some(Status::Connecting) | Some(Status::Joining) => {
                text_mid("Online race", x + w * 0.5, y + 50.0 * s, 40.0 * s, INK);
                let addr = self.client.as_ref().map(|c| c.address.clone()).unwrap_or_default();
                let dots = ".".repeat(1 + (get_time() * 3.0) as usize % 3);
                text_mid(&format!("Connecting to {addr}{dots}"), x + w * 0.5, y + h * 0.45, 24.0 * s, INK_SOFT);
                focus.begin(1);
                if button_back(focus, "Cancel", ix, y + h - rh - pad, iw, rh, CREAM) || focus.back {
                    self.disconnect();
                }
                LobbyAction::None
            }
            Some(_) => self.lobby(focus, ix, y, iw, h, rh, car, paint, allowed_cars),
        }
    }

    fn connect_form(&mut self, focus: &mut Focus, ix: f32, y: f32, iw: f32, h: f32, rh: f32, car: &mut usize, paint: &mut usize, allowed: &[usize]) -> LobbyAction {
        let s = scale();
        let cx = ix + iw * 0.5;
        text_mid("Online race", cx, y + 50.0 * s, 40.0 * s, INK);
        text_mid("Race friends on a server one of you hosts (rally_server)", cx, y + 88.0 * s, 18.0 * s, INK_SOFT);
        focus.begin(6);
        let mut fy = y + 118.0 * s;
        let mut ed = self.editing;
        text_field(focus, &mut ed, 0, "Server", &mut self.address, 64, ix, fy, iw, rh);
        fy += rh + 12.0 * s;
        text_field(focus, &mut ed, 1, "Your name", &mut self.name, 16, ix, fy, iw, rh);
        self.editing = ed;
        fy += rh + 12.0 * s;
        let pos = allowed.iter().position(|c| c == car).unwrap_or(0);
        let d = selector(focus, "Car", car_name(*car as u8), ix, fy, iw, rh, PEACH);
        if d != 0 && !allowed.is_empty() {
            *car = allowed[(pos as i32 + d).rem_euclid(allowed.len() as i32) as usize];
        }
        fy += rh + 12.0 * s;
        let d = selector_swatch(focus, "Paint", "", Some(rgb3(PAINTS[*paint % PAINTS.len()])), ix, fy, iw, rh, PEACH);
        *paint = (*paint as i32 + d).rem_euclid(PAINTS.len() as i32) as usize;
        fy += rh + 18.0 * s;
        if let Some(m) = &self.message {
            text_mid(m, cx, fy + 10.0 * s, 18.0 * s, BERRY);
        }
        let hint = ["Host: run rally_server on any PC, then share its address.", "Same Wi-Fi: its LAN IP. Over the internet: a Tailscale/ZeroTier IP,", "or the host's public IP with port 27420 (TCP + UDP) forwarded."];
        for (i, l) in hint.iter().enumerate() {
            text_mid(l, cx, fy + 40.0 * s + i as f32 * 22.0 * s, 16.0 * s, INK_SOFT);
        }
        let by = y + h - rh - 28.0 * s;
        let bw = (iw - 16.0 * s) * 0.5;
        let back = button_back(focus, "Back", ix, by, bw, rh, CREAM);
        let go = button(focus, "Connect", ix + bw + 16.0 * s, by, bw, rh, MINT);
        if back || (focus.back && self.editing.is_none()) {
            set_typing(false);
            return LobbyAction::Back;
        }
        if go {
            set_typing(false);
            self.editing = None;
            self.message = None;
            let (addr, name) = (self.address.trim().to_string(), self.name.trim().to_string());
            crate::settings::save_online(&addr, &name);
            self.client = Some(NetClient::connect(&addr, &name, *car as u8, *paint as u8));
            self.sent_car = Some((*car as u8, *paint as u8));
        }
        LobbyAction::None
    }

    fn lobby(&mut self, focus: &mut Focus, ix: f32, y: f32, iw: f32, h: f32, rh: f32, car: &mut usize, paint: &mut usize, allowed: &[usize]) -> LobbyAction {
        let s = scale();
        let cx = ix + iw * 0.5;
        let c = self.client.as_ref().unwrap();
        let leader = c.is_leader();
        text_mid(&c.server_name, cx, y + 46.0 * s, 34.0 * s, INK);
        text_mid(&format!("{}  -  {} / {} racers  -  ping {:.0} ms", c.address, c.players.len(), MAX_PLAYERS, c.rtt_ms), cx, y + 80.0 * s, 16.0 * s, INK_SOFT);
        // player list
        let mut py = y + 100.0 * s;
        let row = 44.0 * s;
        for p in &c.players {
            let me = p.id == c.my_id;
            rounded_rect(ix, py, iw, row - 6.0 * s, 12.0 * s, if me { mix(CREAM, BUTTER, 0.45) } else { CREAM });
            circle(ix + 22.0 * s, py + (row - 6.0 * s) * 0.5, 10.0 * s, rgb3(PAINTS[p.paint as usize % PAINTS.len()]));
            let name = if p.id == c.leader { format!("{}  (leader)", p.name) } else { p.name.clone() };
            text(&name, ix + 44.0 * s, mid_baseline(py + (row - 6.0 * s) * 0.5, 20.0 * s), 20.0 * s, INK);
            text(car_name(p.car), ix + iw * 0.45, mid_baseline(py + (row - 6.0 * s) * 0.5, 18.0 * s), 18.0 * s, INK_SOFT);
            let tag = if p.racing { "racing".to_string() } else if p.ready { "ready!".to_string() } else { format!("{} ms  {}", p.ping_ms, if p.udp { "UDP" } else { "TCP" }) };
            text_right(&tag, ix + iw - 16.0 * s, mid_baseline(py + (row - 6.0 * s) * 0.5, 18.0 * s), 18.0 * s, if p.ready { MINT } else { INK_SOFT });
            py += row;
        }
        let race_running = c.race_running;
        let settings = c.settings.clone();
        let my_ready = c.me().is_some_and(|m| m.ready);
        let all_ready = c.players.iter().all(|p| p.ready || p.id == c.leader);
        let mut fy = y + 100.0 * s + row * MAX_PLAYERS as f32 + 8.0 * s;
        let line = match &settings {
            Some(st) => race_summary(st),
            None if leader => "Pick the race with \"Race setup\"".into(),
            None => "Waiting for the leader to pick a race".into(),
        };
        text_mid(&line, cx, fy + 8.0 * s, 17.0 * s, INK);
        if race_running {
            text_mid("A race is running - you'll join the next one", cx, fy + 32.0 * s, 17.0 * s, BERRY);
        }
        fy += 46.0 * s;
        focus.begin(if leader { 6 } else { 4 });
        let half = (iw - 16.0 * s) * 0.5;
        let pos = allowed.iter().position(|x| x == car).unwrap_or(0);
        let d = selector(focus, "Car", car_name(*car as u8), ix, fy, half, rh, PEACH);
        if d != 0 && !allowed.is_empty() {
            *car = allowed[(pos as i32 + d).rem_euclid(allowed.len() as i32) as usize];
        }
        let d = selector_swatch(focus, "Paint", "", Some(rgb3(PAINTS[*paint % PAINTS.len()])), ix + half + 16.0 * s, fy, half, rh, PEACH);
        *paint = (*paint as i32 + d).rem_euclid(PAINTS.len() as i32) as usize;
        fy += rh + 12.0 * s;
        let mut action = LobbyAction::None;
        if leader {
            if button(focus, "Race setup", ix, fy, half, rh, SKY) {
                action = LobbyAction::PickRace;
            }
            let d = selector(focus, "Cars", if self.contact { "Contact" } else { "Ghosts" }, ix + half + 16.0 * s, fy, half, rh, SKY);
            if d != 0 {
                self.contact = !self.contact;
                if let Some(mut st) = settings.clone() {
                    st.contact = self.contact;
                    self.send(&ClientMsg::SetSettings(st));
                }
            }
            fy += rh + 12.0 * s;
        }
        let by = fy.max(y + h - rh - 28.0 * s).min(y + h - rh - 12.0 * s);
        let leave = button_back(focus, "Leave", ix, by, half, rh, CREAM);
        if leader {
            let label = if settings.is_none() { "Pick a race first" } else if race_running { "Race running" } else if all_ready { "Start race!" } else { "Start (not all ready)" };
            if button(focus, label, ix + half + 16.0 * s, by, half, rh, PEACH) && settings.is_some() && !race_running {
                self.send(&ClientMsg::StartRace);
            }
        } else if button(focus, if my_ready { "Ready! (undo)" } else { "I'm ready" }, ix + half + 16.0 * s, by, half, rh, MINT) {
            self.send(&ClientMsg::SetReady(!my_ready));
        }
        if leave || focus.back {
            self.disconnect();
            return LobbyAction::Back;
        }
        self.sync_car(*car, *paint);
        action
    }
}
