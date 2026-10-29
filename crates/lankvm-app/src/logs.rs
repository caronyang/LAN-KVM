use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use lankvm_platform::logfile;

use crate::state::{config_dir, hostname, App, PairedPeer};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub const LOG_PORT: u16 = 45873;
const MAGIC: &[u8] = b"LKLOG1\n";

/// Only the tail of each log goes into a bundle. The files rotate at 5 MB, and
/// the interesting part of a bug report is always the end, so shipping whole
/// files only made the transfer slower and the receiver hungrier.
const BUNDLE_FILE_CAP: u64 = 2 * 1024 * 1024;

/// Hard ceiling for a whole bundle, both when sending and when accepting one.
const BUNDLE_MAX_BYTES: u64 = 64 * 1024 * 1024;

const HEADER_CAP: usize = 512;

/// Generations of each log file that may exist on disk (`name.log` .. `.3`).
const LOG_GENERATIONS: u32 = 3;

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
    let result = send_bundle_from(&dir, host, port, token);
    let _ = fs::remove_dir_all(&dir);
    result
}

fn send_bundle_from(dir: &Path, host: &str, port: u16, token: &str) -> Result<()> {
    let length = bundle_len(dir)?;

    let mut stream =
        TcpStream::connect((host, port)).with_context(|| format!("connect {host}:{port}"))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let header = format!(
        "POST /upload HTTP/1.1\r\nHost: {host}:{port}\r\nX-Lankvm-Token: {token}\r\nContent-Type: application/octet-stream\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n",
    );
    stream.write_all(header.as_bytes())?;
    let written = write_bundle(dir, &mut stream)?;
    if written != length {
        bail!("sent {written} of {length} bundle bytes");
    }
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut response = [0u8; 128];
    let _ = stream.read(&mut response);
    Ok(())
}

fn fetch_peer(app: &Arc<App>, dir: &Path, peer: &PairedPeer) -> Result<bool> {
    let listener = TcpListener::bind(("0.0.0.0", LOG_PORT))
        .with_context(|| format!("bind log port {LOG_PORT}"))?;
    listener.set_nonblocking(true)?;

    let token = format!("{:x}", now_nanos());
    crate::net::send_logs_request(app, peer, &token, LOG_PORT);

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                // accept() on a nonblocking listener returns a nonblocking
                // socket on Windows; switch it back before reading.
                stream.set_nonblocking(false)?;
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
                if length as u64 > BUNDLE_MAX_BYTES {
                    respond(&mut stream, "413 Payload Too Large");
                    continue;
                }
                if body.len() > length {
                    body.truncate(length);
                }

                let peer_dir = dir.join(format!("peer-{}", safe_name(&peer.name)));
                fs::create_dir_all(&peer_dir)?;
                let remaining = length - body.len();
                // Stream the rest straight into the bundle files: a log bundle
                // never has to be held in memory as a whole.
                let unpacked = {
                    let mut tail = (&mut stream).take(remaining as u64);
                    unpack_stream(io::Cursor::new(body).chain(&mut tail), &peer_dir)
                };
                match unpacked {
                    Ok(consumed) if consumed == length as u64 => {}
                    Ok(consumed) => {
                        respond(&mut stream, "400 Bad Request");
                        bail!("bundle size mismatch: got {consumed} of {length} bytes");
                    }
                    Err(err) => {
                        respond(&mut stream, "400 Bad Request");
                        return Err(err);
                    }
                }
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

/// Bundle entries in a stable order, with their real sizes.
fn bundle_entries(dir: &Path) -> Result<Vec<(String, PathBuf, u64)>> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());
    let mut files = Vec::new();
    for entry in entries {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.contains('\t') || name.contains('\n') {
            continue;
        }
        let len = entry.metadata()?.len();
        files.push((name, path, len));
    }
    Ok(files)
}

fn bundle_header(name: &str, len: u64) -> String {
    format!("{name}\t{len}\n")
}

/// Size of the packed bundle without building it in memory.
fn bundle_len(dir: &Path) -> Result<u64> {
    let mut total = MAGIC.len() as u64;
    for (name, _, len) in bundle_entries(dir)? {
        total += bundle_header(&name, len).len() as u64 + len;
    }
    if total > BUNDLE_MAX_BYTES {
        bail!("log bundle too large: {total} bytes");
    }
    Ok(total)
}

/// Write the bundle to any sink, streaming one file at a time.
fn write_bundle(dir: &Path, out: &mut impl Write) -> Result<u64> {
    out.write_all(MAGIC)?;
    let mut written = MAGIC.len() as u64;
    for (name, path, len) in bundle_entries(dir)? {
        let header = bundle_header(&name, len);
        out.write_all(header.as_bytes())?;
        written += header.len() as u64;

        let mut file = fs::File::open(&path).with_context(|| format!("open {name}"))?;
        let copied = io::copy(&mut file, out)?;
        if copied != len {
            bail!("{name} changed while packing ({copied} of {len} bytes)");
        }
        written += copied;
    }
    out.flush()?;
    Ok(written)
}

/// Read a `name\tlength\n` framed bundle and write the files into `dir`.
/// Returns how many bytes were consumed.
fn unpack_stream<R: Read>(reader: R, dir: &Path) -> Result<u64> {
    let mut reader = Counted {
        inner: reader,
        bytes: 0,
    };

    let mut magic = [0u8; MAGIC.len()];
    reader.read_exact(&mut magic).context("read bundle magic")?;
    if magic != MAGIC {
        bail!("bad bundle magic");
    }

    while let Some(line) = read_header_line(&mut reader)? {
        let header = String::from_utf8(line).context("bundle header is not UTF-8")?;
        let Some((name, length)) = header.rsplit_once('\t') else {
            bail!("bad bundle header");
        };
        let length: u64 = length.parse().context("bad bundle length")?;
        if length > BUNDLE_MAX_BYTES {
            bail!("bundle entry {name} is too large");
        }
        let path = dir.join(safe_name(name));
        let mut file =
            fs::File::create(&path).with_context(|| format!("create {}", path.display()))?;
        let copied = io::copy(&mut (&mut reader).take(length), &mut file)?;
        if copied != length {
            bail!("truncated bundle entry {name}");
        }
    }

    Ok(reader.bytes)
}

/// One header line, or `None` at a clean end of bundle. The final entry is not
/// required to be followed by another header.
fn read_header_line<R: Read>(reader: &mut R) -> Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte)? {
            0 => {
                if line.is_empty() {
                    return Ok(None);
                }
                bail!("unterminated bundle header");
            }
            _ => {
                if byte[0] == b'\n' {
                    return Ok(Some(line));
                }
                line.push(byte[0]);
                if line.len() > HEADER_CAP {
                    bail!("bundle header too long");
                }
            }
        }
    }
}

struct Counted<R> {
    inner: R,
    bytes: u64,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.bytes += n as u64;
        Ok(n)
    }
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
            match copy_tail(&from, &dir.join(name), BUNDLE_FILE_CAP) {
                Ok(note) => notes.push(format!("{name}: {note}")),
                Err(err) => notes.push(format!("FAILED {name}: {err:#}")),
            }
        } else {
            notes.push(format!("missing {name} ({})", from.display()));
        }
    };

    copy(config_dir().join("app.log"), "app.log");
    copy(config_dir().join("engine.log"), "engine.log");
    copy(config_dir().join("app.json"), "app.json");
    copy(program_data_logs().join("service.log"), "service.log");
    copy(program_data_logs().join("session.log"), "session.log");
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
        ("wlan", "netsh.exe", vec!["wlan", "show", "interfaces"]),
        (
            "wlan_networks",
            "netsh.exe",
            vec!["wlan", "show", "networks", "mode=bssid"],
        ),
        (
            "wlan_power",
            "powershell.exe",
            vec![
                "-NoProfile",
                "-Command",
                "Get-NetAdapterPowerManagement -Name * -ErrorAction SilentlyContinue | Select-Object Name,AllowComputerToTurnOffDevice | Format-List",
            ],
        ),
        ("service", "sc.exe", vec!["query", "LAN-KVM"]),
        ("udp", "netstat.exe", vec!["-ano", "-p", "UDP"]),
        (
            "firewall_discovery",
            "netsh.exe",
            vec![
                "advfirewall",
                "firewall",
                "show",
                "rule",
                "name=LAN-KVM Discovery",
            ],
        ),
        (
            "firewall_control",
            "netsh.exe",
            vec![
                "advfirewall",
                "firewall",
                "show",
                "rule",
                "name=LAN-KVM Control",
            ],
        ),
        (
            "firewall_data",
            "netsh.exe",
            vec![
                "advfirewall",
                "firewall",
                "show",
                "rule",
                "name=LAN-KVM Data",
            ],
        ),
        (
            "firewall_logs",
            "netsh.exe",
            vec!["advfirewall", "firewall", "show", "rule", "name=LAN-KVM Logs"],
        ),
        (
            "firewall_clipboard",
            "netsh.exe",
            vec![
                "advfirewall",
                "firewall",
                "show",
                "rule",
                "name=LAN-KVM Clipboard",
            ],
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

/// Copy at most `cap` bytes from the end of `from`.
fn copy_tail(from: &Path, to: &Path, cap: u64) -> Result<String> {
    let mut source = fs::File::open(from).with_context(|| format!("open {}", from.display()))?;
    let total = source.metadata()?.len();
    let start = total.saturating_sub(cap);
    if start > 0 {
        source.seek(SeekFrom::Start(start))?;
        // Drop the partial first line so the bundle starts on a whole record.
        let mut byte = [0u8; 1];
        let mut skipped = 0usize;
        while skipped < 8 * 1024 && source.read(&mut byte)? == 1 {
            skipped += 1;
            if byte[0] == b'\n' {
                break;
            }
        }
    }
    let mut out = fs::File::create(to).with_context(|| format!("create {}", to.display()))?;
    let copied = io::copy(&mut source, &mut out)?;
    Ok(if start > 0 {
        format!("truncated, kept last {copied} of {total} bytes")
    } else {
        format!("copied {copied} bytes")
    })
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

fn program_data_logs() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join("LAN-KVM")
        .join("logs")
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

/// Every log file LAN-KVM may have written, including rotated generations.
pub fn log_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for base in [
        config_dir().join("app.log"),
        config_dir().join("engine.log"),
        program_data_logs().join("service.log"),
        program_data_logs().join("session.log"),
    ] {
        paths.push(base.clone());
        for index in 1..=LOG_GENERATIONS {
            paths.push(logfile::rotated_path(&base, index));
        }
    }
    paths
}

/// Existing log files with their sizes, sorted by name.
pub fn log_files() -> Vec<(String, u64)> {
    let mut files = Vec::new();
    for path in log_paths() {
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let name = file_name(&path);
        files.push((name, meta.len()));
    }
    files.sort();
    files
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Delete every log file. Rotated copies are included so "clear" really means
/// zero bytes on disk, not just an empty current file.
pub fn clear(app: &Arc<App>) -> Value {
    // engine.log is the running engine's stdout: deleting it mid-session would
    // silently discard the rest of that run.
    let engine_busy = app
        .session
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some();

    let mut cleared = Vec::new();
    let mut skipped = Vec::new();
    let mut failed = Vec::new();

    for path in log_paths() {
        if !path.exists() {
            continue;
        }
        let name = file_name(&path);
        if engine_busy && name.starts_with("engine.log") {
            skipped.push(name);
            continue;
        }
        match fs::remove_file(&path) {
            Ok(()) => cleared.push(name),
            Err(err) => failed.push(format!("{name}: {err}")),
        }
    }

    if failed.is_empty() {
        json!({"ok": true, "cleared": cleared, "skipped": skipped})
    } else {
        json!({
            "ok": false,
            "cleared": cleared,
            "skipped": skipped,
            "error": failed.join("; "),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn temp_dir(tag: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "lankvm-bundle-{tag}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn bundle_roundtrip_streams_files() {
        let source = temp_dir("pack");
        fs::write(source.join("a.log"), b"hello\n").unwrap();
        fs::write(source.join("b.txt"), b"world").unwrap();

        let len = bundle_len(&source).unwrap();
        let mut packed = Vec::new();
        let written = write_bundle(&source, &mut packed).unwrap();
        assert_eq!(written, len);
        assert_eq!(packed.len() as u64, len);

        let out = temp_dir("unpack");
        let consumed = unpack_stream(Cursor::new(packed), &out).unwrap();
        assert_eq!(consumed, len);
        assert_eq!(fs::read(out.join("a.log")).unwrap(), b"hello\n");
        assert_eq!(fs::read(out.join("b.txt")).unwrap(), b"world");

        fs::remove_dir_all(&source).unwrap();
        fs::remove_dir_all(&out).unwrap();
    }

    #[test]
    fn truncated_bundle_is_rejected() {
        let source = temp_dir("trunc");
        fs::write(source.join("a.log"), vec![b'x'; 4096]).unwrap();
        let mut packed = Vec::new();
        write_bundle(&source, &mut packed).unwrap();
        packed.truncate(packed.len() - 16);

        let out = temp_dir("trunc-out");
        assert!(unpack_stream(Cursor::new(packed), &out).is_err());

        fs::remove_dir_all(&source).unwrap();
        fs::remove_dir_all(&out).unwrap();
    }

    #[test]
    fn copy_tail_keeps_the_end_and_whole_lines() {
        let dir = temp_dir("tail");
        let from = dir.join("big.log");
        fs::write(&from, b"aaaaaaaaaaaaaaaa\nfirst kept?\nlast line\n").unwrap();
        let to = dir.join("small.log");

        let note = copy_tail(&from, &to, 12).unwrap();
        assert!(note.contains("truncated"), "{note}");
        let copied = fs::read(&to).unwrap();
        assert!(copied.ends_with(b"last line\n"));
        assert!(!copied.starts_with(b"a"));

        fs::remove_dir_all(&dir).unwrap();
    }
}
