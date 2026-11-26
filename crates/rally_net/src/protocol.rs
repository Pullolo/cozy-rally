//! Wire protocol shared by the game client and `rally_server`.
//!
//! Transport: one TCP connection per player (TCP_NODELAY) for lobby/control messages, every
//! message a length-prefixed bincode frame (`u32` little-endian length, then the payload).
//! The ~30 Hz car snapshots go over UDP on the same port number (one bincode `UdpMsg` per
//! datagram): a lost snapshot is simply replaced by the next one instead of stalling the stream
//! like TCP would. The client proves who it is with the `udp_token` from `Welcome`; until the
//! server acknowledges its UDP hello (or if UDP is blocked by a firewall) snapshots fall back to
//! the TCP connection, so a race always works.
//!
//! Model: every client simulates its OWN car authoritatively and streams snapshots (state +
//! inputs). Other clients simulate remote cars locally with the received inputs and softly pull
//! them toward the received state (see `sync.rs`). The server is a relay with lobby and race
//! bookkeeping: grid, synchronised start, official finish times.
use rally_core::track::TrackParams;
use serde::{Deserialize, Serialize};

/// Bump whenever a message layout changes; mismatching clients are refused with a clear reason.
pub const PROTOCOL_VERSION: u32 = 5;
pub const DEFAULT_PORT: u16 = 27420;
pub const MAX_PLAYERS: usize = 4;
/// Snapshot send rate per client
pub const SNAPSHOT_HZ: f32 = 30.0;
/// Largest accepted frame (bytes)
pub const MAX_FRAME: usize = 64 * 1024;
/// Countdown length sent with `Go` (seconds)
pub const COUNTDOWN: f32 = 4.0;

pub type PlayerId = u8;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerInfo {
    pub id: PlayerId,
    pub name: String,
    pub car: u8,
    pub paint: u8,
    pub ready: bool,
    pub ping_ms: u16,
    /// Still driving the current race (joined the lobby mid-race = false)
    pub racing: bool,
    /// Snapshots reach this player over UDP (false = TCP fallback)
    pub udp: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RaceSettings {
    pub track: TrackParams,
    pub laps: u32,
    /// Cars collide with each other (false = ghosts: never unfair with lag)
    pub contact: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GridEntry {
    pub id: PlayerId,
    pub name: String,
    pub car: u8,
    pub paint: u8,
}

/// Sent to everyone when the leader starts a race. `grid` order == car slot order in `World`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RaceStart {
    pub race_id: u32,
    pub settings: RaceSettings,
    pub grid: Vec<GridEntry>,
}

/// A car's state at the sender's race clock. Positions are world metres.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CarSnapshot {
    /// Sender's local race session time (seconds, monotonic; not the race clock)
    pub t: f32,
    pub pos: [f32; 3],
    pub rot: [f32; 4],
    pub vel: [f32; 3],
    pub ang_vel: [f32; 3],
    pub throttle: f32,
    pub brake: f32,
    pub steer: f32,
    pub handbrake: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClientMsg {
    Hello { version: u32, name: String, car: u8, paint: u8 },
    SetCar { car: u8, paint: u8 },
    SetReady(bool),
    /// Leader only: race settings shown in the lobby
    SetSettings(RaceSettings),
    /// Leader only
    StartRace,
    /// Track built, waiting at the line
    Loaded { race_id: u32 },
    State { race_id: u32, snap: CarSnapshot },
    Finished { race_id: u32, time: f32 },
    /// Quit the current race (stay in the lobby)
    LeaveRace { race_id: u32 },
    Ping { t: f64, rtt_ms: u16 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServerMsg {
    Welcome { id: PlayerId, server_name: String, udp_token: u64 },
    Refused { reason: String },
    Lobby { players: Vec<PlayerInfo>, leader: PlayerId, settings: Option<RaceSettings>, race_running: bool },
    Start(RaceStart),
    /// Everyone is loaded (or the load timeout passed): begin the countdown now
    Go { race_id: u32, countdown: f32 },
    State { id: PlayerId, snap: CarSnapshot },
    /// Official finish time of a player
    Finished { id: PlayerId, time: f32 },
    /// A player disconnected or quit the race
    Left { id: PlayerId },
    /// Final official results: (id, finish time or None = did not finish), finish order first
    RaceOver { race_id: u32, results: Vec<(PlayerId, Option<f32>)> },
    Pong { t: f64 },
}

/// One UDP datagram (bincode, no framing). Snapshots only; everything else stays on TCP.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum UdpMsg {
    /// client -> server: register / keep the NAT mapping open (sent every few seconds)
    Hello { id: PlayerId, token: u64 },
    /// server -> client: hello accepted, snapshots may use UDP
    Ack,
    /// client -> server: my car
    State { id: PlayerId, token: u64, race_id: u32, snap: CarSnapshot },
    /// server -> client: another player's car
    Relay { id: PlayerId, snap: CarSnapshot },
}

/// Largest datagram we send or accept (snapshots are ~90 bytes)
pub const MAX_DATAGRAM: usize = 1200;
