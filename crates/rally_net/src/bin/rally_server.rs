//! Self-hosted Cozy Rally race server.
//!
//!   rally_server [--port 27420] [--name "Hazel's garage"]
//!
//! Friends connect with your address: LAN IP at home, your Tailscale/ZeroTier IP, or your public
//! IP with TCP port 27420 forwarded on the router.
use rally_net::server::Server;
use rally_net::{DEFAULT_PORT, MAX_PLAYERS, PROTOCOL_VERSION};
use std::time::Duration;

fn main() {
    let mut port = DEFAULT_PORT;
    let mut name = String::from("Cozy Rally server");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" | "-p" => port = args.next().and_then(|p| p.parse().ok()).unwrap_or_else(|| usage("--port needs a number")),
            "--name" | "-n" => name = args.next().unwrap_or_else(|| usage("--name needs a value")),
            "--help" | "-h" => usage(""),
            other => usage(&format!("unknown argument {other}")),
        }
    }
    let mut server = match Server::bind(&format!("0.0.0.0:{port}"), &name) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Can't listen on port {port}: {e}");
            std::process::exit(1);
        }
    };
    println!("Cozy Rally server \"{name}\" (protocol {PROTOCOL_VERSION}, up to {MAX_PLAYERS} players)");
    println!("Listening on TCP port {port}.");
    if let Some(ip) = lan_ip() {
        println!("Friends on your network join with:  {ip}:{port}");
    }
    println!("Over the internet: share your Tailscale/ZeroTier IP, or forward TCP {port} and share your public IP.");
    println!("Ctrl+C to stop.\n");
    loop {
        for line in server.poll() {
            println!("{line}");
        }
        // ~500 Hz poll: snapshots are relayed within a couple of ms
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn usage(err: &str) -> ! {
    if !err.is_empty() {
        eprintln!("error: {err}\n");
    }
    eprintln!("usage: rally_server [--port {DEFAULT_PORT}] [--name \"My server\"]");
    std::process::exit(if err.is_empty() { 0 } else { 2 });
}

/// The LAN address the OS would route outward from (no packet is sent)
fn lan_ip() -> Option<std::net::IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    s.local_addr().ok().map(|a| a.ip())
}
