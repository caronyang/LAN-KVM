//! Cross-machine clipboard sync (text / image / files).
//!
//! Transport is a small length-prefixed postcard protocol over TCP 45875.
//! Only the paired `client` side listens; the paired `master` side connects
//! and retries until the peer is reachable. Frames are bidirectional, so a
//! copy on either machine reaches the other.
//!
//! Echo suppression: after a remote frame is written to the local clipboard,
//! the watcher is acknowledged and a short window suppresses the resulting
//! local change so it is not sent back.

use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use lankvm_clipboard::{ClipboardContent, ClipboardWatcher};
use serde::{Deserialize, Serialize};

use crate::state::{config_dir, App, PairedPeer};

pub const CLIPBOARD_PORT: u16 = 45_875;

const MAGIC: u32 = 0x4C4B_4342; // "LKCB"
const VERSION: u16 = 1;

/// Hard cap for a single frame. Files are bundled per copy operation, so this
/// also caps how much file payload one paste can carry.
const MAX_FRAME: usize = 192 * 1024 * 1024;
const MAX_FILES_TOTAL: usize = 128 * 1024 * 1024;
const MAX_FILES: usize = 200;
const ECHO_WINDOW: Duration = Duration::from_millis(1_500);
const HEARTBEAT: Duration = Duration::from_secs(10);

/// Bumped for every accepted connection; older sessions see a mismatch and
/// exit, so a reconnect never leaves two writers fighting over the clipboard.
static CONN_GEN: AtomicU64 = AtomicU64::new(0);
static SUPPRESS_UNTIL_MS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Serialize, Deserialize)]
struct Hello {
    magic: u32,
    version: u16,
    id: String,
    name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireFile {
    name: String,
    data: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
enum Frame {
    Ping,
    Text(String),
    Image {
        width: u32,
        height: u32,
        png: Vec<u8>,
    },
    Files(Vec<WireFile>),
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn suppressed() -> bool {
    now_ms() < SUPPRESS_UNTIL_MS.load(Ordering::SeqCst)
}

fn suppress_for(window: Duration) {
    SUPPRESS_UNTIL_MS.store(now_ms() + window.as_millis() as u64, Ordering::SeqCst);
}

pub fn start(app: Arc<App>) {
    thread::spawn(move || supervisor(app));
}

/// Decides which side of the link this machine is and starts the matching
/// worker. Roles come from the paired entry: `master` connects, `client`
/// listens.
fn supervisor(app: Arc<App>) {
    let mut listener_started = false;
    let mut last_error: Option<Instant> = None;

    loop {
        let (role, peer) = {
            let config = app.config.lock().unwrap_or_else(|e| e.into_inner());
            let master = config
                .paired
                .iter()
                .find(|p| p.role == "master")
                .cloned();
            let client = config
                .paired
                .iter()
                .find(|p| p.role == "client")
                .cloned();
            match (master, client) {
                (Some(peer), _) => ("master", Some(peer)),
                (None, Some(peer)) => ("client", Some(peer)),
                _ => ("none", None),
            }
        };

        match (role, peer) {
            ("master", Some(peer)) => {
                match run_connector(&app, &peer) {
                    Ok(()) => last_error = None,
                    Err(err) => {
                        let due = last_error
                            .map(|t| t.elapsed() > Duration::from_secs(60))
                            .unwrap_or(true);
                        if due {
                            app.log(format!("clipboard link failed: {err:#}"));
                            last_error = Some(Instant::now());
                        }
                    }
                }
                thread::sleep(Duration::from_secs(2));
            }
            ("client", Some(_)) => {
                if !listener_started {
                    listener_started = true;
                    let app = app.clone();
                    thread::spawn(move || match run_listener(app.clone()) {
                        Ok(()) => {}
                        Err(err) => app.log(format!("clipboard listener stopped: {err:#}")),
                    });
                }
                thread::sleep(Duration::from_secs(1));
            }
            _ => thread::sleep(Duration::from_secs(1)),
        }
    }
}

fn run_listener(app: Arc<App>) -> Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", CLIPBOARD_PORT))
        .with_context(|| format!("bind clipboard TCP {CLIPBOARD_PORT}"))?;
    app.log(format!("clipboard listening on TCP {CLIPBOARD_PORT}"));
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let app = app.clone();
                thread::spawn(move || {
                    if let Err(err) = accept_session(&app, stream) {
                        app.log(format!("clipboard accept failed: {err:#}"));
                    }
                });
            }
            Err(err) => {
                app.log(format!("clipboard accept error: {err}"));
                thread::sleep(Duration::from_millis(500));
            }
        }
    }
    Ok(())
}

fn run_connector(app: &Arc<App>, peer: &PairedPeer) -> Result<()> {
    let addr = format!("{}:{}", peer.ip, CLIPBOARD_PORT);
    let sock_addr: SocketAddr = addr
        .parse()
        .with_context(|| format!("clipboard peer address {addr}"))?;
    let mut stream = TcpStream::connect_timeout(&sock_addr, Duration::from_millis(1500))
        .with_context(|| format!("connect clipboard peer {addr}"))?;
    stream.set_nodelay(true).ok();
    handshake(app, &mut stream)?;
    // The master seeds the link with its current clipboard; the client only
    // sends changes from now on.
    run_session(app.clone(), stream, true)
}

fn accept_session(app: &Arc<App>, mut stream: TcpStream) -> Result<()> {
    stream.set_nodelay(true).ok();
    handshake(app, &mut stream)?;
    run_session(app.clone(), stream, false)
}

fn handshake(app: &Arc<App>, stream: &mut TcpStream) -> Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;

    let (id, name) = {
        let config = app.config.lock().unwrap_or_else(|e| e.into_inner());
        (config.id.clone(), config.name.clone())
    };
    let hello = Hello {
        magic: MAGIC,
        version: VERSION,
        id,
        name,
    };
    write_frame(stream, &postcard::to_allocvec(&hello)?)?;

    let bytes = read_frame(stream)?;
    let remote: Hello = postcard::from_bytes(&bytes).context("decode clipboard hello")?;
    if remote.magic != MAGIC {
        bail!("clipboard hello magic mismatch");
    }
    if remote.version != VERSION {
        bail!("clipboard protocol version {} != {VERSION}", remote.version);
    }
    if remote.id.is_empty() {
        bail!("clipboard hello without id");
    }
    let paired = {
        let config = app.config.lock().unwrap_or_else(|e| e.into_inner());
        config.peer(&remote.id).is_some()
    };
    if !paired {
        bail!("clipboard connection from unpaired peer {}", remote.id);
    }
    app.log(format!(
        "clipboard link up with {} ({})",
        remote.name, remote.id
    ));
    Ok(())
}

fn run_session(app: Arc<App>, mut stream: TcpStream, send_initial: bool) -> Result<()> {
    let gen = CONN_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let alive = Arc::new(AtomicBool::new(true));
    let watcher = Arc::new(Mutex::new(ClipboardWatcher::new()));
    // Fingerprint of the content the peer last sent us. Writing it to the
    // clipboard bumps the local sequence number, so without this the other
    // side would see our echo and bounce the same content back forever.
    let last_remote = Arc::new(AtomicU64::new(0));

    let mut reader_stream = stream.try_clone().context("clone clipboard socket")?;
    // Blocking reads: the reader is unblocked by shutting the socket down when
    // the writer ends, so partial frames can never be abandoned mid-stream.
    reader_stream.set_read_timeout(None)?;
    let shutdown_stream = stream.try_clone().context("clone clipboard socket")?;

    let reader_app = app.clone();
    let reader_alive = alive.clone();
    let reader_watcher = watcher.clone();
    let reader_remote = last_remote.clone();
    let reader = thread::spawn(move || {
        loop {
            if CONN_GEN.load(Ordering::SeqCst) != gen {
                break;
            }
            match read_frame(&mut reader_stream) {
                Ok(bytes) => {
                    match apply_frame(&reader_app, &reader_watcher, &bytes) {
                        Ok(Some(fingerprint)) => {
                            reader_remote.store(fingerprint, Ordering::SeqCst)
                        }
                        Ok(None) => {}
                        Err(err) => reader_app.log(format!("clipboard apply failed: {err:#}")),
                    }
                }
                Err(_) => break,
            }
        }
        reader_alive.store(false, Ordering::SeqCst);
    });

    stream.set_write_timeout(Some(Duration::from_secs(30)))?;

    if send_initial {
        if let Some(content) = current_clipboard() {
            watcher
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .acknowledge();
            if let Ok(frame) = build_frame(&app, &content) {
                if write_frame(&mut stream, &postcard::to_allocvec(&frame)?).is_ok() {
                    app.log(format!(
                        "clipboard sent: {}",
                        lankvm_clipboard::content_summary(&content)
                    ));
                }
            }
        }
    }

    let mut last_sent = Instant::now();
    let mut last_sent_fp = 0u64;
    loop {
        if !alive.load(Ordering::SeqCst) || CONN_GEN.load(Ordering::SeqCst) != gen {
            break;
        }
        let change = {
            let mut guard = watcher.lock().unwrap_or_else(|e| e.into_inner());
            guard.poll()
        };
        if let Some(content) = change {
            let fp = fingerprint(&content);
            if suppressed() || fp == last_remote.load(Ordering::SeqCst) || fp == last_sent_fp {
                continue;
            }
            match build_frame(&app, &content) {
                Ok(frame) => match postcard::to_allocvec(&frame) {
                    Ok(bytes) => {
                        if let Err(err) = write_frame(&mut stream, &bytes) {
                            app.log(format!("clipboard send failed: {err}"));
                            break;
                        }
                        last_sent = Instant::now();
                        last_sent_fp = fp;
                        app.log(format!(
                            "clipboard sent: {}",
                            lankvm_clipboard::content_summary(&content)
                        ));
                    }
                    Err(err) => app.log(format!("clipboard encode failed: {err}")),
                },
                Err(err) => app.log(format!("clipboard skipped: {err:#}")),
            }
        }

        // Keeps the link warm and lets a dead peer be detected without a
        // clipboard change.
        if last_sent.elapsed() > HEARTBEAT {
            last_sent = Instant::now();
            if write_frame(&mut stream, &postcard::to_allocvec(&Frame::Ping)?).is_err() {
                break;
            }
        }

        thread::sleep(Duration::from_millis(120));
    }

    alive.store(false, Ordering::SeqCst);
    let _ = shutdown_stream.shutdown(Shutdown::Both);
    let _ = reader.join();
    app.log("clipboard link down");
    Ok(())
}

fn read_frame(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).map_err(anyhow::Error::new)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        bail!("clipboard frame too large ({len} bytes)");
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).map_err(anyhow::Error::new)?;
    Ok(buf)
}

fn write_frame(stream: &mut TcpStream, bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_FRAME {
        bail!("clipboard frame too large ({} bytes)", bytes.len());
    }
    stream
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .context("write clipboard frame length")?;
    stream.write_all(bytes).context("write clipboard frame")?;
    stream.flush().ok();
    Ok(())
}

fn current_clipboard() -> Option<ClipboardContent> {
    lankvm_clipboard::read_supported()
        .ok()
        .filter(|content| match content {
            ClipboardContent::Text(text) => !text.is_empty(),
            ClipboardContent::Files(files) => !files.is_empty(),
            ClipboardContent::Image { png, .. } => !png.is_empty(),
        })
}

fn build_frame(app: &Arc<App>, content: &ClipboardContent) -> Result<Frame> {
    Ok(match content {
        ClipboardContent::Text(text) => Frame::Text(text.clone()),
        ClipboardContent::Image {
            width,
            height,
            png,
        } => Frame::Image {
            width: *width,
            height: *height,
            png: png.clone(),
        },
        ClipboardContent::Files(paths) => {
            let mut total = 0usize;
            let mut files = Vec::new();
            for path in paths {
                if files.len() >= MAX_FILES {
                    app.log("clipboard files: too many entries; sending the first 200");
                    break;
                }
                let meta = fs::metadata(path)
                    .with_context(|| format!("stat clipboard file {}", path.display()))?;
                if !meta.is_file() {
                    app.log(format!(
                        "clipboard files: skipping non-file {}",
                        path.display()
                    ));
                    continue;
                }
                if total + meta.len() as usize > MAX_FILES_TOTAL {
                    app.log(format!(
                        "clipboard files: skipping {} (size limit)",
                        path.display()
                    ));
                    continue;
                }
                let data = fs::read(path)
                    .with_context(|| format!("read clipboard file {}", path.display()))?;
                total += data.len();
                files.push(WireFile {
                    name: sanitize_name(path),
                    data,
                });
            }
            if files.is_empty() {
                bail!("no readable files");
            }
            Frame::Files(files)
        }
    })
}

fn apply_frame(
    app: &Arc<App>,
    watcher: &Arc<Mutex<ClipboardWatcher>>,
    bytes: &[u8],
) -> Result<Option<u64>> {
    let frame: Frame = postcard::from_bytes(bytes).context("decode clipboard frame")?;
    let content = match frame {
        Frame::Ping => return Ok(None),
        Frame::Text(text) => ClipboardContent::Text(text),
        Frame::Image {
            width,
            height,
            png,
        } => ClipboardContent::Image {
            width,
            height,
            png,
        },
        Frame::Files(files) => ClipboardContent::Files(write_incoming_files(&files)?),
    };
    let fp = fingerprint(&content);

    let mut last_err = None;
    for _ in 0..3 {
        match lankvm_clipboard::write_supported(&content) {
            Ok(()) => {
                suppress_for(ECHO_WINDOW);
                watcher
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .acknowledge();
                app.log(format!(
                    "clipboard received: {}",
                    lankvm_clipboard::content_summary(&content)
                ));
                return Ok(Some(fp));
            }
            Err(err) => {
                last_err = Some(err);
                thread::sleep(Duration::from_millis(80));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("write clipboard failed")))
}

/// Stable, cheap identity of clipboard content, used only for echo detection.
fn fingerprint(content: &ClipboardContent) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match content {
        ClipboardContent::Text(text) => {
            1u8.hash(&mut hasher);
            text.hash(&mut hasher);
        }
        ClipboardContent::Image { png, .. } => {
            2u8.hash(&mut hasher);
            png.hash(&mut hasher);
        }
        ClipboardContent::Files(files) => {
            3u8.hash(&mut hasher);
            for path in files {
                path.file_name().hash(&mut hasher);
                if let Ok(meta) = fs::metadata(path) {
                    meta.len().hash(&mut hasher);
                }
            }
        }
    }
    hasher.finish()
}

fn write_incoming_files(files: &[WireFile]) -> Result<Vec<PathBuf>> {
    let root = config_dir().join("clipboard");
    cleanup_clipboard_dirs(&root);
    let dir = root.join(now_ms().to_string());
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;

    let mut paths = Vec::new();
    for (index, file) in files.iter().enumerate() {
        if index >= MAX_FILES {
            break;
        }
        let name = sanitize_str(&file.name);
        let name = if name.is_empty() {
            format!("file-{index}")
        } else {
            name
        };
        let path = unique_path(&dir, &name);
        fs::write(&path, &file.data)
            .with_context(|| format!("write clipboard file {}", path.display()))?;
        paths.push(path);
    }
    if paths.is_empty() {
        bail!("empty clipboard file bundle");
    }
    Ok(paths)
}

fn cleanup_clipboard_dirs(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let cutoff = now_ms().saturating_sub(2 * 24 * 60 * 60 * 1000);
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(stamp) = name.to_str().and_then(|s| s.parse::<u64>().ok()) else {
            continue;
        };
        if stamp < cutoff {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

fn sanitize_name(path: &Path) -> String {
    sanitize_str(
        &path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
    )
}

fn sanitize_str(value: &str) -> String {
    value
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
        .filter(|c| !c.is_control())
        .take(180)
        .collect::<String>()
        .trim()
        .to_string()
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let ext = path
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    for index in 1..1000 {
        let candidate = dir.join(format!("{stem} ({index}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("file-{}{ext}", now_ms()))
}
