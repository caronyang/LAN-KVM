use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

use crate::state::{config_dir, hostname, App, PairedPeer};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub const LOG_PORT: u16 = 45873;
const MAGIC: &[u8] = b"LKLOG1\n";

pub fn collect(app: &Arc<App>) -> Result<PathBuf> {
    let dir = create_bundle(app, desktop_base())?;

    let peer = app
        .config
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .paired
        .first()
        .cloned();
    if let Some(peer) = peer {
        let online = app.peer_by_id(&peer.id).is_some();
        if online {
            match fetch_peer(app, &dir, &peer) {
                Ok(true) => app.log(format!("remote logs fetched from {}", peer.name)),
                Ok(false) => {
                    let _ = fs::write(
                        dir.join("peer-error.txt"),
                        format!("no response from {} ({})\n", peer.name, peer.ip),
                    );
                    app.log(format!("remote logs timed out: {}", peer.name));
                }
                Err(err) => {
                    let _ = fs::write(dir.join("peer-error.txt"), format!("{err:#}\n"));
                    app.log(format!("remote logs failed: {err:#}"));
                }
            }
        } else {
            let _ = fs::write(
                dir.join("peer-error.txt"),
                format!("{} is not online (discovery)\n", peer.name),
            );
        }
    }

    open_explorer(&dir);
    app.log(format!("logs collected to {}", dir.display()));
    Ok(dir)
}

/// Build a bundle and push it to another LAN-KVM app (used by the peer).
pub fn send_bundle(app: &Arc<App>, host: &str, port: u16, token: &str) -> Result<()> {
    let dir = create_bundle(
        app,
        std::env::temp_dir().join(format!("lankvm-remote-logs-{}", now_nanos())),
    )?;
    let body = pack_dir(&dir)?;
    let _ = fs::remove_dir_all(&dir);

    let mut stream = TcpStream::connect((host, port))
        .with_context(|| format!("connect {host}:{port}"))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;
    let header = format!(
        "POST /upload HTTP/1.1\r\nHost: {host}:{port}\r\nX-Lankvm-Token: {token}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(&body)?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut response = [0u8; 128];
    let _ = stream.read(&mut response);
    Ok(())
}

fn fetch_peer(app: &Arc<App>, dir: &PathBuf, peer: &PairedPeer) -> Result<bool> {
    let listener = TcpListener::bind(("0.0.0.0", LOG_PORT))
        .with_context(|| format!("bind log port {LOG_PORT}"))?;
    listener.set_nonblocking(true)?;

    let token = format!("{:x}", now_nanos());
    crate::net::send_logs_request(app, peer, &token, LOG_PORT);

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_secs(20)))?;
                let (headers, mut body) = read_headers(&mut stream)?;
                if !token_matches(&headers, &token) {
                    respond(&mut stream, "403 Forbidden");
                    continue;
                }
                let Some(length) = content_length(&headers) else {
                    respond(&mut stream, "411 Length Required");
                    continue;
                };
                while body.len() < length {
                    let mut chunk = vec![0u8; 64 * 1024];
                    let n = stream.read(&mut chunk)?;
                    if n == 0 {
                        break;
                    }
                    body.extend_from_slice(&chunk[..n]);
                }
                body.truncate(length);
                if !body.starts_with(MAGIC) {
                    respond(&mut stream, "400 Bad Request");
                    bail!("peer sent an unknown log bundle");
                }
                let peer_dir = dir.join(format!("peer-{}", safe_name(&peer.name)));
                fs::create_dir_all(&peer_dir)?;
                unpack_bundle(&body, &peer_dir)?;
                respond(&mut stream, "200 OK");
                return Ok(true);
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Ok(false);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(err) => return Err(err).context("accept log upload"),
        }
    }
}

fn token_matches(headers: &str, token: &str) -> bool {
    headers.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("x-lankvm-token:") {
            value.trim() == token
        } else {
            false
        }
    })
}

fn content_length(headers: &str) -> Option<usize> {
    headers.lines().find_map(|line| {
        let lower = line.to_ascii_lowercase();
        lower
            .strip_prefix("content-length:")
            .and_then(|value| value.trim().parse().ok())
    })
}

fn read_headers(stream: &mut TcpStream) -> Result<(String, Vec<u8>)> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            bail!("connection closed before headers");
        }
        buffer.extend_from_slice(&chunk[..n]);
        if let Some(position) = find_subslice(&buffer, b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&buffer[..position]).to_string();
            let body = buffer[position + 4..].to_vec();
            return Ok((headers, body));
        }
        if buffer.len() > 64 * 1024 {
            bail!("headers too large");
        }
    }
}

fn respond(stream: &mut TcpStream, status: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK"
    );
}

fn pack_dir(dir: &PathBuf) -> Result<Vec<u8>> {
    let mut output = MAGIC.to_vec();
    let mut entries: Vec<_> = fs::read_dir(dir)?.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.contains('\t') || name.contains('\n') {
            continue;
        }
        let data = fs::read(&path)?;
        output.extend_from_slice(format!("{name}\t{}\n", data.len()).as_bytes());
        output.extend_from_slice(&data);
    }
    Ok(output)
}

fn unpack_bundle(mut data: &[u8], dir: &PathBuf) -> Result<()> {
    if !data.starts_with(MAGIC) {
        bail!("bad bundle magic");
    }
    data = &data[MAGIC.len()..];
    while !data.is_empty() {
        let Some(newline) = data.iter().position(|&b| b == b'\n') else {
            break;
        };
        let header = std::str::from_utf8(&data[..newline])?;
        data = &data[newline + 1..];
        let Some((name, length)) = header.rsplit_once('\t') else {
            bail!("bad bundle header");
        };
        let length: usize = length.parse()?;
        if data.len() < length {
            bail!("truncated bundle");
        }
        let name = safe_name(name);
        fs::write(dir.join(name), &data[..length])?;
        data = &data[length..];
    }
    Ok(())
}

fn safe_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

fn create_bundle(app: &Arc<App>, base: PathBuf) -> Result<PathBuf> {
    let dir = base.join(format!("LAN-KVM-logs-{}-{}", hostname(), now_nanos()));
    fs::create_dir_all(&dir).context("create log directory")?;
    let mut notes = Vec::new();

    let mut copy = |from: PathBuf, name: &str| {
        if from.exists() {
            match fs::copy(&from, dir.join(name)) {
                Ok(_) => notes.push(format!("copied {name}")),
                Err(err) => notes.push(format!("FAILED {name}: {err}")),
            }
        } else {
            notes.push(format!("missing {name} ({})", from.display()));
        }
    };

    copy(config_dir().join("app.log"), "app.log");
    copy(config_dir().join("engine.log"), "engine.log");
    copy(config_dir().join("app.json"), "app.json");
    copy(
        PathBuf::from("C:\\ProgramData\\LAN-KVM\\logs\\service.log"),
        "service.log",
    );
    copy(
        PathBuf::from("C:\\ProgramData\\LAN-KVM\\logs\\session.log"),
        "session.log",
    );
    copy(std::env::temp_dir().join("lankvm-setup.log"), "setup.log");

    let state = app.snapshot();
    let _ = fs::write(
        dir.join("state.json"),
        serde_json::to_string_pretty(&state).unwrap_or_default(),
    );

    let mut system = String::new();
    system.push_str(&format!(
        "host={}\ncollected_at={}\nports=http:{},control:{},data:{},discovery:{}\n",
        hostname(),
        now_nanos(),
        app.http_port,
        app.control_port,
        app.data_port,
        app.discovery_port
    ));
    for (label, program, args) in [
        ("ipconfig", "ipconfig.exe", vec!["/all"]),
        ("service", "sc.exe", vec!["query", "LAN-KVM"]),
        ("udp", "netstat.exe", vec!["-ano", "-p", "UDP"]),
        (
            "firewall_discovery",
            "netsh.exe",
            vec!["advfirewall", "firewall", "show", "rule", "name=LAN-KVM Discovery"],
        ),
        (
            "firewall_control",
            "netsh.exe",
            vec!["advfirewall", "firewall", "show", "rule", "name=LAN-KVM Control"],
        ),
        (
            "firewall_data",
            "netsh.exe",
            vec!["advfirewall", "firewall", "show", "rule", "name=LAN-KVM Data"],
        ),
        (
            "firewall_logs",
            "netsh.exe",
            vec!["advfirewall", "firewall", "show", "rule", "name=LAN-KVM Logs"],
        ),
    ] {
        system.push_str(&format!("\n===== {label} =====\n"));
        match run(program, &args) {
            Ok(text) => system.push_str(&text),
            Err(err) => system.push_str(&format!("error: {err}\n")),
        }
    }
    let _ = fs::write(dir.join("system.txt"), system);

    notes.push("state.json / system.txt written".to_string());
    let mut readme =
        String::from("LAN-KVM log bundle\n\nSend the whole folder to the developer.\n\nFiles:\n");
    for note in &notes {
        readme.push_str(&format!("- {note}\n"));
    }
    let _ = fs::write(dir.join("README.txt"), readme);
    Ok(dir)
}

fn run(program: &str, args: &[&str]) -> Result<String> {
    let mut command = Command::new(program);
    command.args(args);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let output = command.output().context("run command")?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(text)
}

fn open_explorer(dir: &PathBuf) {
    let _ = Command::new("explorer.exe")
        .arg(dir)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

fn desktop_base() -> PathBuf {
    let profile = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let desktop = profile.join("Desktop");
    let onedrive = profile.join("OneDrive").join("Desktop");
    if desktop.exists() {
        desktop
    } else if onedrive.exists() {
        onedrive
    } else {
        profile
    }
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
