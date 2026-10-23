use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_json::{json, Value};
use socket2::{Domain, Protocol, Socket, Type};

use crate::engine;
use crate::state::{save, App, Pairing, PairedPeer, PeerInfo, Session};

pub const DISCOVERY_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 42, 99);
static LAST_PAIR_OPEN: AtomicU64 = AtomicU64::new(0);

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn base(app: &Arc<App>, kind: &str) -> Value {
    let config = app.config.lock().unwrap_or_else(|e| e.into_inner());
    json!({
        "type": kind,
        "id": config.id,
        "name": config.name,
        "control_port": app.control_port,
        "data_port": app.data_port,
    })
}

fn send_to(app: &Arc<App>, value: &Value, ip: IpAddr, port: u16) {
    let Ok(socket) = UdpSocket::bind(("0.0.0.0", 0)) else {
        return;
    };
    let addr = SocketAddr::new(ip, port);
    if let Err(err) = socket.send_to(value.to_string().as_bytes(), addr) {
        app.log(format!("send {addr} failed: {err}"));
    }
}

fn parse_or(ip: &str) -> IpAddr {
    ip.parse().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

fn s(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn p(value: &Value, key: &str) -> u16 {
    value.get(key).and_then(|v| v.as_u64()).unwrap_or(0) as u16
}

pub fn send_public(app: &Arc<App>, value: &Value, ip: &str, port: u16) {
    send_to(app, value, parse_or(ip), port);
}

pub fn pair_request_message(app: &Arc<App>) -> Value {
    base(app, "pair_request")
}

pub fn pair_proof_message(app: &Arc<App>, code: &str) -> Value {
    let mut value = base(app, "pair_proof");
    if let Value::Object(map) = &mut value {
        map.insert("code".to_string(), json!(code));
    }
    value
}

pub fn unpair_message(app: &Arc<App>) -> Value {
    base(app, "unpair")
}

pub fn announce_now(app: &Arc<App>) {
    let Ok(socket) = UdpSocket::bind(("0.0.0.0", 0)) else {
        return;
    };
    let _ = socket.set_broadcast(true);
    let msg = base(app, "discover");
    let port = app.discovery_port;
    let _ = socket.send_to(
        msg.to_string().as_bytes(),
        SocketAddr::new(IpAddr::V4(DISCOVERY_GROUP), port),
    );
    let _ = socket.send_to(
        msg.to_string().as_bytes(),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), port),
    );
}

pub fn start(
    app: Arc<App>,
    discovery_port: u16,
    control_port: u16,
    data_port: u16,
    _http_port: u16,
) -> Result<()> {
    let _ = data_port;
    start_control(app.clone(), control_port)?;
    start_discovery(app.clone(), discovery_port)?;
    start_announce(app.clone(), discovery_port)?;
    start_service_watch(app.clone());
    start_supervisor(app);
    Ok(())
}

fn service_running(app: &Arc<App>) -> bool {
    // The client engine (SYSTEM service) answers P0 Ping on the data port.
    // This is more reliable than parsing localized `sc` output and also
    // proves the injection listener is actually ready.
    let Ok(socket) = UdpSocket::bind(("127.0.0.1", 0)) else {
        return false;
    };
    let _ = socket.set_read_timeout(Some(Duration::from_millis(250)));
    let mut ping = lankvm_net::Packet::new(lankvm_net::PacketKind::Ping, 0);
    ping.a = 0;
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, app.data_port));
    if socket.send_to(&ping.encode(), addr).is_err() {
        return false;
    }
    let mut buf = [0u8; 64];
    match socket.recv_from(&mut buf) {
        Ok((n, _)) => lankvm_net::Packet::decode(&buf[..n])
            .map(|p| p.kind == lankvm_net::PacketKind::Pong && p.d == 1)
            .unwrap_or(false),
        Err(_) => false,
    }
}

fn start_service_watch(app: Arc<App>) {
    thread::spawn(move || loop {
        let running = service_running(&app);
        let previous = app.service_ok.swap(running, Ordering::Relaxed);
        if previous != running {
            app.log(format!(
                "client engine {}",
                if running { "ready (SYSTEM service)" } else { "not ready" }
            ));
            if running {
                // A restarted client engine means the master's remote state is
                // stale. Ask it to stop and reconnect so the cursor is not
                // stuck clipped on the master.
                let has_session = app
                    .session
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .is_some();
                if has_session {
                    let config = app
                        .config
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone();
                    if let Some(peer) = config.paired.iter().find(|p| p.role == "client") {
                        let msg = base(&app, "session_reset");
                        send_to(&app, &msg, parse_or(&peer.ip), peer.control_port);
                    }
                }
            }
        }
        thread::sleep(Duration::from_secs(5));
    });
}

fn start_discovery(app: Arc<App>, port: u16) -> Result<()> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)).into())?;
    socket.join_multicast_v4(&DISCOVERY_GROUP, &Ipv4Addr::UNSPECIFIED)?;
    socket.set_multicast_loop_v4(true)?;
    socket.set_broadcast(true)?;
    socket.set_read_timeout(Some(Duration::from_millis(250)))?;
    let socket: UdpSocket = socket.into();

    thread::spawn(move || {
        let mut buf = [0u8; 2048];
        loop {
            match socket.recv_from(&mut buf) {
                Ok((n, src)) => {
                    if let Ok(value) = serde_json::from_slice::<Value>(&buf[..n]) {
                        handle_discovery(&app, &value, src.ip());
                    }
                }
                Err(err)
                    if matches!(
                        err.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => break,
            }
        }
    });
    Ok(())
}

fn start_control(app: Arc<App>, port: u16) -> Result<()> {
    let socket =
        UdpSocket::bind(("0.0.0.0", port)).with_context(|| format!("bind control UDP {port}"))?;
    socket.set_read_timeout(Some(Duration::from_millis(250)))?;

    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match socket.recv_from(&mut buf) {
                Ok((n, src)) => {
                    if let Ok(value) = serde_json::from_slice::<Value>(&buf[..n]) {
                        handle_control(&app, &value, src.ip());
                    }
                }
                Err(err)
                    if matches!(
                        err.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => break,
            }
        }
    });
    Ok(())
}

fn start_announce(app: Arc<App>, port: u16) -> Result<()> {
    let socket = UdpSocket::bind(("0.0.0.0", 0))?;
    socket.set_broadcast(true)?;
    thread::spawn(move || {
        for _ in 0..5 {
            let msg = base(&app, "discover");
            let _ = socket.send_to(
                msg.to_string().as_bytes(),
                SocketAddr::new(IpAddr::V4(DISCOVERY_GROUP), port),
            );
            let _ = socket.send_to(
                msg.to_string().as_bytes(),
                SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), port),
            );
            thread::sleep(Duration::from_millis(250));
        }
        loop {
            thread::sleep(Duration::from_secs(3));
            let msg = base(&app, "discover");
            let _ = socket.send_to(
                msg.to_string().as_bytes(),
                SocketAddr::new(IpAddr::V4(DISCOVERY_GROUP), port),
            );
            let _ = socket.send_to(
                msg.to_string().as_bytes(),
                SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), port),
            );
            let mut peers = app.peers.lock().unwrap_or_else(|e| e.into_inner());
            peers.retain(|_, peer| peer.last_seen.elapsed() < Duration::from_secs(15));
        }
    });
    Ok(())
}

fn handle_discovery(app: &Arc<App>, value: &Value, src: IpAddr) {
    let id = s(value, "id");
    if id.is_empty() {
        return;
    }
    let own = app
        .config
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .id
        .clone();
    if id == own {
        return;
    }

    let info = PeerInfo {
        id: id.clone(),
        name: s(value, "name"),
        ip: src.to_string(),
        control_port: p(value, "control_port"),
        data_port: p(value, "data_port"),
        last_seen: Instant::now(),
    };
    if info.control_port == 0 || info.data_port == 0 {
        return;
    }
    app.peer_upsert(info);

    if s(value, "type") == "discover" {
        let reply = base(app, "peer");
        send_to(app, &reply, src, p(value, "control_port"));
    }
}

fn handle_control(app: &Arc<App>, value: &Value, src: IpAddr) {
    let kind = s(value, "type");
    let id = s(value, "id");
    let own = app
        .config
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .id
        .clone();
    if id == own {
        return;
    }

    if !id.is_empty() {
        let info = PeerInfo {
            id: id.clone(),
            name: s(value, "name"),
            ip: src.to_string(),
            control_port: p(value, "control_port"),
            data_port: p(value, "data_port"),
            last_seen: Instant::now(),
        };
        if info.control_port != 0 {
            app.peer_upsert(info);
        }
    }

    match kind.as_str() {
        "pair_request" => handle_pair_request(app, value, src),
        "pair_proof" => handle_pair_proof(app, value, src),
        "pair_ok" => handle_pair_ok(app, value, src),
        "pair_fail" => {
            if let Some(pairing) = app
                .pairing
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
            {
                pairing.error = Some("配對碼不正確".to_string());
            }
            app.log("pairing failed: wrong code");
        }
        "unpair" => {
            let mut config = app.config.lock().unwrap_or_else(|e| e.into_inner());
            config.paired.retain(|p| p.id != id);
            save(app.profile.as_deref(), &config);
            drop(config);
            if app
                .session
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .map(|s| s.peer_id == id)
                .unwrap_or(false)
            {
                engine::stop_session(app, false);
            }
            app.log(format!("unpaired by peer {id}"));
        }
        "session_start" => handle_session_start(app, value),
        "session_stop" => {
            let stop = app
                .session
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .map(|s| s.role == "client" && s.peer_id == id)
                .unwrap_or(false);
            if stop {
                engine::stop_session(app, false);
            }
        }
        "logs_request" => {
            let token = s(value, "token");
            let port = p(value, "upload_port");
            let paired = app
                .config
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .peer(&id)
                .is_some();
            if paired && port != 0 && !token.is_empty() {
                let app = app.clone();
                let host = src.to_string();
                thread::spawn(move || {
                    if let Err(err) = crate::logs::send_bundle(&app, &host, port, &token) {
                        app.log(format!("remote log upload failed: {err:#}"));
                    }
                });
            }
        }
        "session_reset" => {
            let is_master = app
                .session
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .map(|s| s.role == "master" && s.peer_id == id)
                .unwrap_or(false);
            if is_master {
                engine::stop_session(app, false);
                *app.suppress_until.lock().unwrap_or_else(|e| e.into_inner()) = None;
                app.log("peer client restarted; master stopped for a clean reconnect");
            }
        }
        "session_ack" => {}
        "peer" => {}
        "discover" => {}
        _ => {}
    }
}

fn handle_pair_request(app: &Arc<App>, value: &Value, src: IpAddr) {
    let id = s(value, "id");
    let name = s(value, "name");
    let config = app.config.lock().unwrap_or_else(|e| e.into_inner()).clone();

    if config.peer(&id).is_some() {
        let reply = base(app, "pair_ok");
        send_to(app, &reply, src, p(value, "control_port"));
        app.log(format!("pair request from already-paired {name}"));
        return;
    }

    let code = format!("{:03}", 100 + (now_nanos() / 1000 % 900) as u32);
    *app.pairing.lock().unwrap_or_else(|e| e.into_inner()) = Some(Pairing {
        peer_id: id.clone(),
        peer_name: name.clone(),
        peer_ip: src.to_string(),
        peer_control_port: p(value, "control_port"),
        peer_data_port: p(value, "data_port"),
        role: "host".to_string(),
        code: Some(code.clone()),
        awaiting_code: true,
        error: None,
        created: Instant::now(),
    });

    let reply = base(app, "pair_challenge");
    send_to(app, &reply, src, p(value, "control_port"));
    app.log(format!(
        "pairing request from {name} {src}; code shown locally"
    ));

    // The host may be running headless at boot. Open its UI once so the code
    // is visible without the user hunting for the app.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    if now.saturating_sub(LAST_PAIR_OPEN.load(Ordering::Relaxed)) > 30_000 {
        LAST_PAIR_OPEN.store(now, Ordering::Relaxed);
        crate::open_browser(&format!("http://127.0.0.1:{}/", app.http_port));
    }
}

fn handle_pair_proof(app: &Arc<App>, value: &Value, src: IpAddr) {
    let id = s(value, "id");
    let supplied = s(value, "code");
    let pairing = app
        .pairing
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();

    let Some(pairing) = pairing else {
        return;
    };
    if pairing.role != "host" || pairing.peer_id != id {
        return;
    }
    if pairing.code.as_deref() != Some(supplied.as_str()) {
        let reply = base(app, "pair_fail");
        send_to(app, &reply, src, pairing.peer_control_port);
        app.log("pairing proof rejected");
        return;
    }

    let mut config = app.config.lock().unwrap_or_else(|e| e.into_inner());
    let edge = config.edge.clone();
    let trigger_px = config.trigger_px;
    let trigger_ms = config.trigger_ms;
    let release_px = config.release_px;
    config.paired.retain(|p| p.id != id);
    config.paired.push(PairedPeer {
        id: id.clone(),
        name: pairing.peer_name.clone(),
        ip: src.to_string(),
        control_port: pairing.peer_control_port,
        data_port: pairing.peer_data_port,
        role: "client".to_string(),
        auto_connect: true,
        edge,
        trigger_px,
        trigger_ms,
        release_px,
    });
    save(app.profile.as_deref(), &config);
    drop(config);

    *app.pairing.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let reply = base(app, "pair_ok");
    send_to(app, &reply, src, pairing.peer_control_port);
    app.log(format!("paired with {} ({id})", pairing.peer_name));
}

fn handle_pair_ok(app: &Arc<App>, value: &Value, src: IpAddr) {
    let id = s(value, "id");
    let name = s(value, "name");
    let mut config = app.config.lock().unwrap_or_else(|e| e.into_inner());
    let edge = config.edge.clone();
    let trigger_px = config.trigger_px;
    let trigger_ms = config.trigger_ms;
    let release_px = config.release_px;
    config.paired.retain(|p| p.id != id);
    config.paired.push(PairedPeer {
        id: id.clone(),
        name: name.clone(),
        ip: src.to_string(),
        control_port: p(value, "control_port"),
        data_port: p(value, "data_port"),
        role: "master".to_string(),
        auto_connect: true,
        edge,
        trigger_px,
        trigger_ms,
        release_px,
    });
    save(app.profile.as_deref(), &config);
    drop(config);

    *app.pairing.lock().unwrap_or_else(|e| e.into_inner()) = None;
    app.log(format!("paired with {name} ({id})"));
}

fn handle_session_start(app: &Arc<App>, value: &Value) {
    let id = s(value, "id");
    let config = app
        .config
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let Some(paired) = config.peer(&id) else {
        app.log(format!("session_start from unpaired {id} ignored"));
        return;
    };
    if paired.role != "client" {
        return;
    }

    let now = Instant::now();
    let mut session = app.session.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = session.as_mut() {
        if existing.role == "client" && existing.peer_id == id && existing.uses_service {
            existing.last_seen = now;
            return;
        }
    }

    if !app.service_ok.load(Ordering::Relaxed) {
        app.log("session_start ignored: SYSTEM service is not running");
        return;
    }

    *session = Some(Session {
        peer_id: id.clone(),
        role: "client".to_string(),
        peer_ip: paired.ip.clone(),
        data_port: app.data_port,
        pid: 0,
        stop_file: PathBuf::new(),
        started: now,
        last_seen: now,
        uses_service: true,
    });
    app.log(format!(
        "client session via SYSTEM service for {} ({})",
        paired.name, paired.ip
    ));
}

pub fn send_session_start(app: &Arc<App>, peer: &PairedPeer, edge: &str) {
    let mut msg = base(app, "session_start");
    if let Value::Object(map) = &mut msg {
        map.insert("edge".to_string(), json!(edge));
        map.insert("trigger_px".to_string(), json!(peer.trigger_px));
        map.insert("trigger_ms".to_string(), json!(peer.trigger_ms));
    }
    send_to(app, &msg, parse_or(&peer.ip), peer.control_port);
}

pub fn send_logs_request(
    app: &Arc<App>,
    peer: &PairedPeer,
    token: &str,
    upload_port: u16,
) {
    let mut msg = base(app, "logs_request");
    if let Value::Object(map) = &mut msg {
        map.insert("token".to_string(), json!(token));
        map.insert("upload_port".to_string(), json!(upload_port));
    }
    send_to(app, &msg, parse_or(&peer.ip), peer.control_port);
}

pub fn send_session_stop(app: &Arc<App>, peer_ip: &str, peer_id: &str) {    let config = app.config.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let port = config
        .peer(peer_id)
        .map(|p| p.control_port)
        .unwrap_or(app.control_port);
    let msg = base(app, "session_stop");
    send_to(app, &msg, parse_or(peer_ip), port);
}

fn start_supervisor(app: Arc<App>) {
    thread::spawn(move || {
        let mut last_keepalive = Instant::now();
        loop {
            thread::sleep(Duration::from_secs(2));

            // Pairing codes expire; otherwise the host UI would stay modal.
            let expired = {
                let mut pairing = app.pairing.lock().unwrap_or_else(|e| e.into_inner());
                let stale = pairing
                    .as_ref()
                    .map(|p| p.created.elapsed() > Duration::from_secs(120))
                    .unwrap_or(false);
                if stale {
                    *pairing = None;
                }
                stale
            };
            if expired {
                app.log("pairing timed out");
            }

            let has_session = app
                .session
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some();
            if has_session {
                let snapshot = app
                    .session
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                let alive = engine::engine_alive(&app);
                let stale_client = snapshot
                    .as_ref()
                    .map(|s| {
                        s.uses_service
                            && (!alive || s.last_seen.elapsed() > Duration::from_secs(15))
                    })
                    .unwrap_or(false);
                if stale_client {
                    *app.session.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    app.log("client session cleared (service down or master gone)");
                    continue;
                }

                // Keep telling the client to run. If its engine died, this
                // restarts it without restarting the master. On the client
                // side it just refreshes last_seen.
                if last_keepalive.elapsed() > Duration::from_secs(5) {
                    last_keepalive = Instant::now();
                    if let Some(session) = snapshot {
                        if session.role == "master" {
                            let config = app
                                .config
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .clone();
                            if let Some(peer) = config.peer(&session.peer_id) {
                                send_session_start(&app, peer, &config.edge);
                            }
                        }
                    }
                }
                continue;
            }

            let suppressed = app
                .suppress_until
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .map(|t| t > Instant::now())
                .unwrap_or(false);
            if suppressed {
                continue;
            }
            if app
                .pairing
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some()
            {
                continue;
            }

            let config = app
                .config
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            if config.paused {
                continue;
            }
            let master = config
                .paired
                .iter()
                .find(|p| p.role == "master" && p.auto_connect)
                .cloned();
            let Some(peer) = master else {
                continue;
            };
            let edge = config.edge.clone();
            let trigger_px = config.trigger_px;
            let trigger_ms = config.trigger_ms;

            let Some(info) = app.peer_by_id(&peer.id) else {
                // Not discovered yet (for example B is sitting at the login
                // screen before its UI starts). Fall back to the address saved
                // at pairing time; the SYSTEM service is already listening.
                if peer.data_port != 0 && !peer.ip.is_empty() {
                    match engine::start_master(
                        &app,
                        &peer.id,
                        &peer.ip,
                        peer.data_port,
                        &edge,
                        trigger_px,
                        trigger_ms,
                    ) {
                        Ok(()) => {
                            send_session_start(&app, &peer, &edge);
                            last_keepalive = Instant::now();
                            app.log(format!(
                                "auto-connect master -> {} ({}, saved address)",
                                peer.name, peer.ip
                            ));
                        }
                        Err(err) => app.log(format!("auto-connect failed: {err:#}")),
                    }
                }
                continue;
            };
            if info.last_seen.elapsed() > Duration::from_secs(10) {
                continue;
            }

            match engine::start_master(
                &app,
                &peer.id,
                &info.ip,
                info.data_port,
                &edge,
                trigger_px,
                trigger_ms,
            ) {
                Ok(()) => {
                    send_session_start(&app, &peer, &edge);
                    last_keepalive = Instant::now();
                    app.log(format!(
                        "auto-connect master -> {} ({})",
                        peer.name, info.ip
                    ));
                }
                Err(err) => app.log(format!("auto-connect failed: {err:#}")),
            }
        }
    });
}
