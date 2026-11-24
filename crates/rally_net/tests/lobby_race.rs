//! End-to-end: a real server on localhost and three clients go through lobby -> start ->
//! load -> go -> snapshots -> finish -> results, plus version refusal and a mid-race drop.
use rally_net::server::Server;
use rally_net::*;
use std::time::{Duration, Instant};

fn pump(server: &mut Server, clients: &mut [&mut NetClient], events: &mut [Vec<NetEvent>]) {
    server.poll();
    for (c, ev) in clients.iter_mut().zip(events.iter_mut()) {
        ev.extend(c.poll());
    }
    std::thread::sleep(Duration::from_millis(2));
}

fn until(server: &mut Server, clients: &mut [&mut NetClient], events: &mut [Vec<NetEvent>], what: &str, mut ok: impl FnMut(&[&mut NetClient], &[Vec<NetEvent>]) -> bool) {
    let t0 = Instant::now();
    while !ok(clients, events) {
        assert!(t0.elapsed() < Duration::from_secs(10), "timed out waiting for {what}");
        pump(server, clients, events);
    }
}

#[test]
fn full_race_flow() {
    let mut server = Server::bind("127.0.0.1:0", "test").unwrap();
    let addr = server.local_addr().unwrap().to_string();
    let mut a = NetClient::connect(&addr, "Alice", 0, 1);
    let mut b = NetClient::connect(&addr, "Bob", 1, 1);
    let mut c = NetClient::connect(&addr, "  ", 2, 1);
    // c behaves like a player behind a firewall that drops UDP: TCP fallback only
    c.udp_allowed = false;
    let mut ev = vec![Vec::new(), Vec::new(), Vec::new()];
    {
        let mut cl = [&mut a, &mut b, &mut c];
        until(&mut server, &mut cl, &mut ev, "lobby of 3", |cl, _| cl.iter().all(|c| c.status == Status::Connected && c.players.len() == 3));
        until(&mut server, &mut cl, &mut ev, "udp for a and b", |cl, _| cl[0].udp_ok && cl[1].udp_ok && cl[0].players.iter().filter(|p| p.udp).count() == 2);
    }
    assert!(server.udp_enabled());
    assert!(!c.udp_ok, "the TCP-only client stays on TCP");
    {
    }
    assert_eq!(server.player_count(), 3);
    // the first to join leads
    let leader = [&a, &b, &c].iter().find(|c| c.is_leader()).map(|c| c.my_id).unwrap();
    assert!(a.players.iter().any(|p| p.name.starts_with("Racer ")), "blank names get a default");
    let (lead, others): (&mut NetClient, Vec<&mut NetClient>) = {
        let all = [&mut a, &mut b, &mut c];
        let mut lead = None;
        let mut others = Vec::new();
        for x in all {
            if x.my_id == leader && lead.is_none() { lead = Some(x) } else { others.push(x) }
        }
        (lead.unwrap(), others)
    };
    let settings = RaceSettings { track: rally_core::track::TrackParams::default(), laps: 2, contact: false };
    // non-leaders can't start or change settings
    let mut others = others;
    others[0].send(&ClientMsg::SetSettings(settings.clone()));
    others[0].send(&ClientMsg::StartRace);
    lead.send(&ClientMsg::SetSettings(settings.clone()));
    lead.send(&ClientMsg::StartRace);
    let mut cl: Vec<&mut NetClient> = vec![lead];
    cl.extend(others);
    until(&mut server, &mut cl, &mut ev, "start", |_, ev| ev.iter().all(|e| e.iter().any(|x| matches!(x, NetEvent::Start(_)))));
    let start = match ev[0].iter().find(|x| matches!(x, NetEvent::Start(_))).unwrap() {
        NetEvent::Start(s) => s.clone(),
        _ => unreachable!(),
    };
    assert_eq!(start.settings, settings);
    assert_eq!(start.grid.len(), 3);
    let mut paints: Vec<u8> = start.grid.iter().map(|g| g.paint).collect();
    paints.sort();
    paints.dedup();
    assert_eq!(paints.len(), 3, "paints are unique on the grid");
    let race_id = start.race_id;
    // nobody gets Go before all are loaded
    cl[0].send(&ClientMsg::Loaded { race_id });
    cl[1].send(&ClientMsg::Loaded { race_id });
    for _ in 0..30 {
        pump(&mut server, &mut cl, &mut ev);
    }
    assert!(!ev.iter().flatten().any(|x| matches!(x, NetEvent::Go { .. })));
    cl[2].send(&ClientMsg::Loaded { race_id });
    until(&mut server, &mut cl, &mut ev, "go", |_, ev| ev.iter().all(|e| e.iter().any(|x| matches!(x, NetEvent::Go { .. }))));
    // snapshots are relayed to the others (UDP or TCP fallback, both directions), not echoed
    for k in 0..3 {
        let snap = CarSnapshot { t: 1.5 + k as f32, pos: [1.0, 2.0, k as f32], throttle: 1.0, ..Default::default() };
        cl[k].send(&ClientMsg::State { race_id, snap });
        let idk = cl[k].my_id;
        until(&mut server, &mut cl, &mut ev, "relay", |_, ev| (0..3).filter(|&j| j != k).all(|j| ev[j].iter().any(|x| *x == NetEvent::State { id: idk, snap })));
        assert!(!ev[k].iter().any(|x| matches!(x, NetEvent::State { id, .. } if *id == idk)), "no echo");
    }
    // player 3 drops mid race; the others are told
    let id2 = cl[2].my_id;
    cl[2].disconnect();
    let mut two: Vec<&mut NetClient> = cl.into_iter().take(2).collect();
    let mut ev2 = vec![std::mem::take(&mut ev[0]), std::mem::take(&mut ev[1])];
    until(&mut server, &mut two, &mut ev2, "left", |_, ev| ev.iter().all(|e| e.iter().any(|x| *x == NetEvent::Left { id: id2 })));
    // both finish -> official results, fastest first
    two[1].send(&ClientMsg::Finished { race_id, time: 61.0 });
    two[0].send(&ClientMsg::Finished { race_id, time: 65.5 });
    until(&mut server, &mut two, &mut ev2, "results", |_, ev| ev.iter().all(|e| e.iter().any(|x| matches!(x, NetEvent::RaceOver { .. }))));
    let results = ev2[0].iter().find_map(|x| if let NetEvent::RaceOver { results, .. } = x { Some(results.clone()) } else { None }).unwrap();
    assert_eq!(results[0], (two[1].my_id, Some(61.0)));
    assert_eq!(results[1], (two[0].my_id, Some(65.5)));
    assert_eq!(results[2], (id2, None));
    until(&mut server, &mut two, &mut ev2, "lobby back", |cl, _| cl.iter().all(|c| !c.race_running && c.players.len() == 2));
}

#[test]
fn refuses_wrong_version_and_bad_address() {
    let mut server = Server::bind("127.0.0.1:0", "test").unwrap();
    let addr = server.local_addr().unwrap();
    // raw old client
    let mut conn = rally_net::conn::Conn::new(std::net::TcpStream::connect(addr).unwrap()).unwrap();
    conn.send(&ClientMsg::Hello { version: PROTOCOL_VERSION + 1, name: "Old".into(), car: 0, paint: 0 });
    conn.flush();
    let t0 = Instant::now();
    let mut got = Vec::new();
    while got.is_empty() && t0.elapsed() < Duration::from_secs(5) {
        server.poll();
        got = conn.recv_all::<ServerMsg>();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(matches!(&got[0], ServerMsg::Refused { reason } if reason.contains("Version")));
    assert_eq!(server.player_count(), 0);
    assert!(rally_net::client::resolve("").is_err());
    assert_eq!(rally_net::client::resolve("127.0.0.1").unwrap()[0].port(), DEFAULT_PORT);
    assert_eq!(rally_net::client::resolve("127.0.0.1:999").unwrap()[0].port(), 999);
}
