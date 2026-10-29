#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod clipboard_sync;
mod engine;
mod http;
mod logs;
mod net;
mod state;
mod tray;

use std::net::TcpStream;
use std::process::Command;
use std::sync::Arc;

pub const DEFAULT_HTTP_PORT: u16 = 45_880;
pub const DEFAULT_CONTROL_PORT: u16 = 45_871;
pub const DEFAULT_DATA_PORT: u16 = 45_872;
pub const DEFAULT_DISCOVERY_PORT: u16 = 45_870;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--engine") {
        engine::run_engine(args.into_iter());
    }

    let mut http_port = DEFAULT_HTTP_PORT;
    let mut control_port = DEFAULT_CONTROL_PORT;
    let mut data_port = DEFAULT_DATA_PORT;
    let mut discovery_port = DEFAULT_DISCOVERY_PORT;
    let mut silent = false;
    let mut name: Option<String> = None;
    let mut profile: Option<String> = None;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--silent" => silent = true,
            "--http-port" => {
                http_port = iter
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(http_port)
            }
            "--control-port" => {
                control_port = iter
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(control_port)
            }
            "--data-port" => {
                data_port = iter
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(data_port)
            }
            "--discovery-port" => {
                discovery_port = iter
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(discovery_port)
            }
            "--name" => name = iter.next().cloned(),
            "--profile" => profile = iter.next().cloned(),
            _ => {}
        }
    }

    let url = format!("http://127.0.0.1:{http_port}/");

    // Another instance already serving the UI: just open it.
    if TcpStream::connect(("127.0.0.1", http_port)).is_ok() {
        if !silent {
            open_browser(&url);
        }
        return;
    }

    let (mut config, _existed) = state::load_or_create(profile.as_deref());
    if let Some(name) = name {
        config.name = name;
    }
    if config.name.trim().is_empty() {
        config.name = state::hostname();
    }
    state::save(profile.as_deref(), &config);

    let app = Arc::new(state::App::new(
        config,
        http_port,
        control_port,
        data_port,
        discovery_port,
        profile.clone(),
    ));
    app.log(format!(
        "LAN-KVM app starting {} (http={http_port} control={control_port} data={data_port} discovery={discovery_port})",
        app.config
            .lock()
            .map(|c| c.name.clone())
            .unwrap_or_default()
    ));

    if let Err(err) = net::start(
        app.clone(),
        discovery_port,
        control_port,
        data_port,
        http_port,
    ) {
        app.log(format!("network start failed: {err:#}"));
    }

    clipboard_sync::start(app.clone());

    let server_app = app.clone();
    std::thread::spawn(move || {
        if let Err(err) = http::serve(server_app.clone(), http_port) {
            server_app.log(format!("UI server failed: {err:#}"));
        }
    });

    if !silent {
        open_browser(&url);
    }

    tray::run(app);
}

pub fn open_browser(url: &str) {
    let _ = Command::new("explorer").arg(url).spawn();
}
