//! Identity announcements from the SYSTEM session agent.
//!
//! The user app only starts after login, but the session agent runs at the
//! login screen. It reads the app's persistent id from the user's app.json and
//! periodically announces it, so a paired peer can find this machine (and its
//! new DHCP address) before anyone logs in.

use std::fs;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime};

use serde_json::{json, Value};

use crate::logger::log;

const DISCOVERY_PORT: u16 = 45_870;
const DISCOVERY_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 42, 99);
const CONTROL_PORT: u16 = 45_871;

pub fn start(stop: Arc<AtomicBool>, data_port: u16) {
    thread::spawn(move || {
        let socket = match UdpSocket::bind(("0.0.0.0", 0)) {
            Ok(socket) => socket,
            Err(err) => {
                log("session", format!("identity announce bind failed: {err}"));
                return;
            }
        };
        let _ = socket.set_broadcast(true);

        let mut identity = load_identity();
        let mut logged_id = String::new();
        let mut tick = 0u32;

        loop {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            if tick % 10 == 0 {
                identity = load_identity();
            }
            tick = tick.wrapping_add(1);

            if let Some((id, name)) = &identity {
                let message = json!({
                    "type": "peer",
                    "id": id,
                    "name": name,
                    "control_port": CONTROL_PORT,
                    "data_port": data_port,
                    "service": true,
                })
                .to_string();
                let _ = socket.send_to(
                    message.as_bytes(),
                    SocketAddr::from((DISCOVERY_GROUP, DISCOVERY_PORT)),
                );
                let _ = socket.send_to(
                    message.as_bytes(),
                    SocketAddr::from((Ipv4Addr::BROADCAST, DISCOVERY_PORT)),
                );
                if logged_id != *id {
                    log(
                        "session",
                        format!(
                            "identity announce id={id} name={name} control={CONTROL_PORT} data={data_port}"
                        ),
                    );
                    logged_id = id.clone();
                }
            }

            thread::sleep(Duration::from_secs(3));
        }
    });
}

fn load_identity() -> Option<(String, String)> {
    let users = fs::read_dir("C:\\Users").ok()?;
    let mut best: Option<(SystemTime, String, String)> = None;

    for entry in users.flatten() {
        let path = entry
            .path()
            .join("AppData")
            .join("Local")
            .join("LAN-KVM")
            .join("app.json");
        let Some((modified, id, name)) = read_candidate(&path) else {
            continue;
        };
        if best
            .as_ref()
            .map(|(current, _, _)| modified > *current)
            .unwrap_or(true)
        {
            best = Some((modified, id, name));
        }
    }

    best.map(|(_, id, name)| (id, name))
}

fn read_candidate(path: &Path) -> Option<(SystemTime, String, String)> {
    let text = fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let id = value.get("id")?.as_str()?.to_string();
    let name = value
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or("LAN-KVM")
        .to_string();
    let paired = value
        .get("paired")
        .and_then(|value| value.as_array())
        .map(|paired| !paired.is_empty())
        .unwrap_or(false);
    if !paired {
        return None;
    }
    let modified = fs::metadata(path).and_then(|meta| meta.modified()).ok()?;
    Some((modified, id, name))
}
