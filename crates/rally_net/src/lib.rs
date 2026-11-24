//! Cozy Rally online multiplayer.
//!
//! * `protocol` - messages shared by client and server
//! * `conn`     - non-blocking framed TCP
//! * `server`   - lobby / race bookkeeping / relay (run by the `rally_server` binary)
//! * `client`   - the game's connection
//! * `sync`     - remote car correction model
pub mod client;
pub mod conn;
pub mod protocol;
pub mod server;
pub mod sync;

pub use client::{NetClient, NetEvent, Status};
pub use protocol::*;
