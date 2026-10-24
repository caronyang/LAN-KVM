use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tiny_http::{Header, Method, Response, Server};

use crate::engine;
use crate::net;
use crate::state::{save, App, Pairing};

const UI_HTML: &str = include_str!("ui.html");

pub fn serve(app: Arc<App>, port: u16) -> Result<()> {
    let server = Server::http(("127.0.0.1", port)).map_err(|e| anyhow!("http bind: {e}"))?;
    app.log(format!("UI http://127.0.0.1:{port}"));

    for mut request in server.incoming_requests() {
        let url = request.url().to_string();
        let method = request.method().clone();

        let mut body = String::new();
        let _ = std::io::Read::read_to_string(&mut request.as_reader(), &mut body);
        let payload: Value = serde_json::from_str(&body).unwrap_or(Value::Null);

        let (content_type, text) = if method == Method::Get && (url == "/" || url == "/index.html")
        {
            ("text/html; charset=utf-8", UI_HTML.to_string())
        } else if method == Method::Get && url == "/api/state" {
            (
                "application/json; charset=utf-8",
                state_json(&app).to_string(),
            )
        } else if method == Method::Post {
            let result = route(&app, &url, &payload);
            ("application/json; charset=utf-8", result.to_string())
        } else {
            (
                "application/json; charset=utf-8",
                json!({"ok": false, "error": "not found"}).to_string(),
            )
        };

        let header = Header::from_bytes("Content-Type", content_type)
            .unwrap_or_else(|_| Header::from_bytes("Content-Type", "text/plain").unwrap());
        let _ = request.respond(
            Response::from_string(text)
                .with_header(header)
                .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap()),
        );
    }
    Ok(())
}

fn state_json(app: &Arc<App>) -> Value {
    let mut value = app.snapshot();
    let engine_log = std::fs::read_to_string(engine::engine_log_path()).unwrap_or_default();
    let tail: Vec<&str> = engine_log.lines().rev().take(25).collect();
    if let Value::Object(map) = &mut value {
        map.insert(
            "engine_log".to_string(),
            json!(tail.into_iter().map(|s| s.to_string()).collect::<Vec<_>>()),
        );
    }
    value
}

fn route(app: &Arc<App>, url: &str, payload: &Value) -> Value {
    match url {
        "/api/scan" => {
            net::announce_now(app);
            json!({"ok": true})
        }
        "/api/pair" => {
            let peer_id = payload
                .get("peer_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let Some(peer) = app.peer_by_id(peer_id) else {
                return json!({"ok": false, "error": "找不到這台電腦，請重新掃描"});
            };
            let config = app.config.lock().unwrap_or_else(|e| e.into_inner());
            if config.peer(peer_id).is_some() {
                return json!({"ok": false, "error": "已經配對"});
            }
            drop(config);

            if peer.control_port == 0 {
                return json!({"ok": false, "error": "對方沒有回報控制埠"});
            }
            let msg = net::pair_request_message(app);
            net::send_public(app, &msg, &peer.ip, peer.control_port);
            *app.pairing.lock().unwrap_or_else(|e| e.into_inner()) = Some(Pairing {
                peer_id: peer.id.clone(),
                peer_name: peer.name.clone(),
                peer_ip: peer.ip.clone(),
                peer_control_port: peer.control_port,
                peer_data_port: peer.data_port,
                role: "guest".to_string(),
                code: None,
                awaiting_code: true,
                error: None,
                created: Instant::now(),
            });
            app.log(format!(
                "pairing requested with {} ({})",
                peer.name, peer.ip
            ));
            json!({"ok": true})
        }
        "/api/pair/code" => {
            let code = payload.get("code").and_then(|v| v.as_str()).unwrap_or("");
            let pairing = app
                .pairing
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let Some(pairing) = pairing else {
                return json!({"ok": false, "error": "沒有進行中的配對"});
            };
            if pairing.role != "guest" {
                return json!({"ok": false, "error": "這台是顯示配對碼的一方"});
            }
            if code.len() != 3 || !code.chars().all(|c| c.is_ascii_digit()) {
                return json!({"ok": false, "error": "請輸入 3 位數字"});
            }
            let msg = net::pair_proof_message(app, code);
            net::send_public(app, &msg, &pairing.peer_ip, pairing.peer_control_port);
            app.log("pairing code sent");
            json!({"ok": true})
        }
        "/api/pair/cancel" => {
            *app.pairing.lock().unwrap_or_else(|e| e.into_inner()) = None;
            json!({"ok": true})
        }
        "/api/unpair" => {
            let peer_id = payload
                .get("peer_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let removed = {
                let mut config = app.config.lock().unwrap_or_else(|e| e.into_inner());
                let before = config.paired.len();
                config.paired.retain(|p| p.id != peer_id);
                save(app.profile.as_deref(), &config);
                before != config.paired.len()
            };
            if let Some(peer) = app.peer_by_id(peer_id) {
                let msg = net::unpair_message(app);
                net::send_public(app, &msg, &peer.ip, peer.control_port);
            }
            if app
                .session
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .map(|s| s.peer_id == peer_id)
                .unwrap_or(false)
            {
                engine::stop_session(app, false);
            }
            *app.suppress_until.lock().unwrap_or_else(|e| e.into_inner()) = None;
            app.log("unpaired");
            json!({"ok": removed})
        }
        "/api/session/start" => {
            let peer_id = payload
                .get("peer_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let edge = payload
                .get("edge")
                .and_then(|v| v.as_str())
                .unwrap_or("right")
                .to_string();
            if app
                .session
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some()
            {
                return json!({"ok": false, "error": "已有連線，請先停止目前連線"});
            }
            let (peer, trigger_px, trigger_ms) = {
                let mut config = app.config.lock().unwrap_or_else(|e| e.into_inner());
                if config.peer(peer_id).is_none() {
                    return json!({"ok": false, "error": "尚未配對"});
                }
                if let Some(target) = config.peer_mut(peer_id) {
                    target.edge = edge.clone();
                }
                config.edge = edge.clone();
                let trigger_px = config.trigger_px;
                let trigger_ms = config.trigger_ms;
                save(app.profile.as_deref(), &config);
                (
                    config.peer(peer_id).cloned().unwrap(),
                    trigger_px,
                    trigger_ms,
                )
            };
            if peer.role != "master" {
                return json!({"ok": false, "error": "這台是 client，請由主控端發起"});
            }
            let (peer_ip, peer_data_port) = match app.peer_by_id(peer_id) {
                Some(info) => (info.ip, info.data_port),
                None if !peer.ip.is_empty() && peer.data_port != 0 => {
                    (peer.ip.clone(), peer.data_port)
                }
                None => {
                    return json!({"ok": false, "error": "對方目前不在線上，且沒有可用的配對位址"});
                }
            };
            if let Err(err) = engine::start_master(
                app,
                &peer.id,
                &peer_ip,
                peer_data_port,
                &edge,
                trigger_px,
                trigger_ms,
            ) {
                app.log(format!("start session failed: {err:#}"));
                return json!({"ok": false, "error": format!("{err:#}")});
            }
            net::send_session_start(app, &peer, &edge);
            *app.suppress_until.lock().unwrap_or_else(|e| e.into_inner()) = None;
            app.log(format!("session started -> {}", peer.name));
            json!({"ok": true})
        }
        "/api/session/stop" => {
            engine::stop_session(app, true);
            *app.suppress_until.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(Instant::now() + Duration::from_secs(60));
            json!({"ok": true})
        }
        "/api/logs" => match crate::logs::collect(app) {
            Ok(path) => json!({"ok": true, "path": path.display().to_string()}),
            Err(err) => json!({"ok": false, "error": format!("{err:#}")}),
        },
        "/api/autoconnect" => {
            let peer_id = payload
                .get("peer_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let enabled = payload
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let mut config = app.config.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(peer) = config.peer_mut(peer_id) {
                peer.auto_connect = enabled;
            }
            save(app.profile.as_deref(), &config);
            json!({"ok": true})
        }
        "/api/settings" => {
            let mut config = app.config.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(edge) = payload.get("edge").and_then(|v| v.as_str()) {
                if edge == "left" || edge == "right" {
                    config.edge = edge.to_string();
                }
            }
            if let Some(v) = payload.get("trigger_px").and_then(|v| v.as_f64()) {
                config.trigger_px = (v as f32).clamp(4.0, 200.0);
            }
            if let Some(v) = payload.get("trigger_ms").and_then(|v| v.as_u64()) {
                config.trigger_ms = v.clamp(80, 2000);
            }
            if let Some(v) = payload.get("release_px").and_then(|v| v.as_i64()) {
                config.release_px = (v as i32).clamp(1, 100);
            }
            if let Some(v) = payload.get("paused").and_then(|v| v.as_bool()) {
                config.paused = v;
            }
            if let Some(v) = payload.get("lang").and_then(|v| v.as_str()) {
                if v == "en" || v == "zh" {
                    config.lang = v.to_string();
                }
            }
            save(app.profile.as_deref(), &config);
            json!({"ok": true})
        }
        "/api/quit" => {
            engine::stop_session(app, true);
            app.log("quit requested");
            thread::spawn(|| {
                thread::sleep(Duration::from_millis(300));
                std::process::exit(0);
            });
            json!({"ok": true})
        }
        _ => json!({"ok": false, "error": "unknown endpoint"}),
    }
}
