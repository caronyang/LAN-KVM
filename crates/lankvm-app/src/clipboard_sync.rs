//! Cross-machine clipboard sync (text / image / files / rich formats).
//!
//! Transport is a small length-prefixed postcard protocol over TCP 45875.
//! Only the paired `client` side listens; the paired `master` side connects
//! and retries until the peer is reachable. Frames are bidirectional, so a
//! copy on either machine reaches the other.
//!
//! Rich formats (`HTML Format`, `Rich Text Format`, `XML Spreadsheet`) are
//! carried next to the plain text, which is what lets a copied Excel range
//! paste as a table instead of a block of text. Peers that predate the rich
//! frame simply fail to decode it and keep receiving text frames.
//!
//! Echo suppression: after a remote frame is written to the local clipboard,
//! the watcher is acknowledged and a short window suppresses the resulting
//! local change so it is not sent back.

use std::collections::HashSet;
use std::fs;
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
/// A copied folder becomes one tree frame. Entries include directories, so
/// the limit is higher than the flat file limit but still bounded.
const MAX_TREE_ENTRIES: usize = 5000;
const MAX_TREE_DEPTH: usize = 32;
/// Excel's HTML can be surprisingly large because it inlines style rules.
const MAX_RICH_FORMAT: usize = 48 * 1024 * 1024;
const MAX_RICH_TOTAL: usize = 64 * 1024 * 1024;
const HEARTBEAT: Duration = Duration::from_secs(10);

/// Bumped for every accepted connection; older sessions see a mismatch and
/// exit, so a reconnect never leaves two writers fighting over the clipboard.
static CONN_GEN: AtomicU64 = AtomicU64::new(0);

/// True while a remote frame is being written to the local clipboard. The
/// writer must not consume the resulting sequence change during this window.
static APPLYING: AtomicBool = AtomicBool::new(false);
/// Sequence number produced by our own clipboard write. Exactly this one change
/// is ignored; a user copy right after it still goes through.
static SUPPRESSED_SEQ: AtomicU64 = AtomicU64::new(u64::MAX);
/// Fingerprint and time of the last frame we applied. Guards against Windows
/// or a clipboard viewer re-announcing the same content right after our write
/// (a user re-copy later is not affected because the window is short).
static LAST_APPLIED: Mutex<Option<(u64, Instant)>> = Mutex::new(None);
const REANNOUNCE_WINDOW: Duration = Duration::from_secs(3);

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
struct WireFormat {
    name: String,
    data: Vec<u8>,
}

/// One item of a copied directory tree. `path` is relative to the bundle root
/// and uses `/` separators; `data` is `None` for a directory.
#[derive(Debug, Serialize, Deserialize)]
struct WireEntry {
    path: String,
    data: Option<Vec<u8>>,
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
    /// Plain text plus rich clipboard formats, so the peer can paste with
    /// formatting (Excel tables, Word styles). Added after `Files`, so the
    /// wire tags of the existing variants do not move.
    Rich {
        text: String,
        formats: Vec<WireFormat>,
    },
    /// Copied folder(s) with their internal layout preserved. Sent instead of
    /// `Files` when the selection contains a directory.
    Tree(Vec<WireEntry>),
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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
            let master = config.paired.iter().find(|p| p.role == "master").cloned();
            let client = config.paired.iter().find(|p| p.role == "client").cloned();
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

    let mut reader_stream = stream.try_clone().context("clone clipboard socket")?;
    // Blocking reads: the reader is unblocked by shutting the socket down when
    // the writer ends, so partial frames can never be abandoned mid-stream.
    reader_stream.set_read_timeout(None)?;
    let shutdown_stream = stream.try_clone().context("clone clipboard socket")?;

    let reader_app = app.clone();
    let reader_alive = alive.clone();
    let reader_watcher = watcher.clone();
    let reader = thread::spawn(move || {
        loop {
            if CONN_GEN.load(Ordering::SeqCst) != gen {
                break;
            }
            match read_frame(&mut reader_stream) {
                Ok(bytes) => {
                    if let Err(err) = apply_frame(&reader_app, &reader_watcher, &bytes) {
                        reader_app.log(format!("clipboard apply failed: {err:#}"));
                    }
                }
                Err(_) => break,
            }
        }
        reader_alive.store(false, Ordering::SeqCst);
    });

    stream.set_write_timeout(Some(Duration::from_secs(30)))?;

    let mut last_sent = Instant::now();
    let mut last_sent_fp = 0u64;

    if send_initial {
        if let Some(content) = current_clipboard() {
            watcher
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .acknowledge();
            if let Ok(frame) = build_frame(&app, &content) {
                last_sent_fp = frame_fingerprint(&frame);
                if write_frame(&mut stream, &postcard::to_allocvec(&frame)?).is_ok() {
                    app.log(format!(
                        "clipboard sent (initial): {}",
                        frame_summary(&frame)
                    ));
                }
            }
        }
    }

    let mut last_sent_at = Instant::now();
    loop {
        if !alive.load(Ordering::SeqCst) || CONN_GEN.load(Ordering::SeqCst) != gen {
            break;
        }

        // Do not consume the sequence change while our own apply is in flight.
        if APPLYING.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(30));
            continue;
        }
        // Ignore exactly the change our own clipboard write produced. A user
        // copy immediately afterwards has a newer sequence and is still sent.
        let seq = lankvm_clipboard::sequence() as u64;
        if seq == SUPPRESSED_SEQ.load(Ordering::SeqCst) {
            watcher
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .acknowledge();
            thread::sleep(Duration::from_millis(120));
            continue;
        }

        let change = {
            let mut guard = watcher.lock().unwrap_or_else(|e| e.into_inner());
            guard.poll()
        };
        if let Some(content) = change {
            // The sequence changed between our flag check and the poll: treat
            // it as our own write rather than echoing it back.
            if APPLYING.load(Ordering::SeqCst) {
                watcher
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .acknowledge();
                thread::sleep(Duration::from_millis(60));
                continue;
            }
            match build_frame(&app, &content) {
                Ok(frame) => {
                    let fp = frame_fingerprint(&frame);
                    if recently_applied(fp)
                        || (fp == last_sent_fp && last_sent_at.elapsed() < REANNOUNCE_WINDOW)
                    {
                        watcher
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .acknowledge();
                        thread::sleep(Duration::from_millis(60));
                        continue;
                    }
                    match postcard::to_allocvec(&frame) {
                        Ok(bytes) => {
                            if let Err(err) = write_frame(&mut stream, &bytes) {
                                app.log(format!("clipboard send failed: {err}"));
                                break;
                            }
                            last_sent = Instant::now();
                            last_sent_fp = fp;
                            last_sent_at = Instant::now();
                            app.log(format!("clipboard sent: {}", frame_summary(&frame)));
                        }
                        Err(err) => app.log(format!("clipboard encode failed: {err}")),
                    }
                }
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
            ClipboardContent::Rich { text, formats } => !text.is_empty() || !formats.is_empty(),
        })
}

fn build_frame(app: &Arc<App>, content: &ClipboardContent) -> Result<Frame> {
    Ok(match content {
        ClipboardContent::Text(text) => Frame::Text(text.clone()),
        ClipboardContent::Image { width, height, png } => Frame::Image {
            width: *width,
            height: *height,
            png: png.clone(),
        },
        ClipboardContent::Files(paths) => {
            let has_directory = paths.iter().any(|path| {
                fs::symlink_metadata(path)
                    .map(|meta| meta.is_dir())
                    .unwrap_or(false)
            });
            if has_directory {
                let mut log = |message: String| app.log(message);
                Frame::Tree(build_tree_entries(paths, &mut log)?)
            } else {
                build_file_frame(app, paths)?
            }
        }
        ClipboardContent::Rich { text, formats } => {
            let mut total = 0usize;
            let mut kept = Vec::new();
            for format in formats {
                if !lankvm_clipboard::is_rich_format(&format.name) {
                    continue;
                }
                if format.data.is_empty() {
                    continue;
                }
                if format.data.len() > MAX_RICH_FORMAT || total + format.data.len() > MAX_RICH_TOTAL
                {
                    app.log(format!(
                        "clipboard rich: skipping {} ({} bytes, size limit)",
                        format.name,
                        format.data.len()
                    ));
                    continue;
                }
                total += format.data.len();
                kept.push(WireFormat {
                    name: format.name.clone(),
                    data: format.data.clone(),
                });
            }
            if kept.is_empty() {
                Frame::Text(text.clone())
            } else {
                Frame::Rich {
                    text: text.clone(),
                    formats: kept,
                }
            }
        }
    })
}

fn build_file_frame(app: &Arc<App>, paths: &[PathBuf]) -> Result<Frame> {
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
        let data =
            fs::read(path).with_context(|| format!("read clipboard file {}", path.display()))?;
        total += data.len();
        files.push(WireFile {
            name: sanitize_name(path),
            data,
        });
    }
    if files.is_empty() {
        bail!("no readable files");
    }
    Ok(Frame::Files(files))
}

/// Walks the selected directories (or files) into a flat list of relative
/// entries. Reparse points are skipped so a junction cannot loop the scan.
fn build_tree_entries(paths: &[PathBuf], log: &mut dyn FnMut(String)) -> Result<Vec<WireEntry>> {
    let mut used_roots: HashSet<String> = HashSet::new();
    let mut entries: Vec<WireEntry> = Vec::new();
    let mut total = 0usize;

    for path in paths {
        if entries.len() >= MAX_TREE_ENTRIES {
            log("clipboard tree: entry limit reached; sending a partial selection".to_string());
            break;
        }
        if path.file_name().is_none() {
            log(format!(
                "clipboard tree: skipping drive root {}",
                path.display()
            ));
            continue;
        }
        let meta = match fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(err) => {
                log(format!(
                    "clipboard tree: cannot stat {}: {err}",
                    path.display()
                ));
                continue;
            }
        };
        if meta.file_type().is_symlink() {
            log(format!("clipboard tree: skipping link {}", path.display()));
            continue;
        }
        let root = unique_root_name(path, &mut used_roots);
        if meta.is_dir() {
            if is_directory_link(&meta) {
                log(format!(
                    "clipboard tree: skipping directory link {}",
                    path.display()
                ));
                continue;
            }
            collect_dir(path, &root, 1, &mut entries, &mut total, log);
        } else if meta.is_file() {
            let size = meta.len() as usize;
            if size > MAX_FILES_TOTAL || total.saturating_add(size) > MAX_FILES_TOTAL {
                log(format!(
                    "clipboard tree: skipping {} (size limit)",
                    path.display()
                ));
                continue;
            }
            match fs::read(path) {
                Ok(data) => {
                    if total.saturating_add(data.len()) > MAX_FILES_TOTAL {
                        log(format!(
                            "clipboard tree: skipping {} (size limit)",
                            path.display()
                        ));
                        continue;
                    }
                    total += data.len();
                    entries.push(WireEntry {
                        path: root,
                        data: Some(data),
                    });
                }
                Err(err) => log(format!(
                    "clipboard tree: cannot read {}: {err}",
                    path.display()
                )),
            }
        } else {
            log(format!(
                "clipboard tree: skipping special file {}",
                path.display()
            ));
        }
    }

    if entries.is_empty() {
        bail!("no readable files in the selection");
    }
    Ok(entries)
}

fn collect_dir(
    dir: &Path,
    rel: &str,
    depth: usize,
    entries: &mut Vec<WireEntry>,
    total: &mut usize,
    log: &mut dyn FnMut(String),
) {
    if entries.len() >= MAX_TREE_ENTRIES {
        return;
    }
    if depth > MAX_TREE_DEPTH {
        log(format!(
            "clipboard tree: depth limit reached at {}",
            dir.display()
        ));
        return;
    }
    entries.push(WireEntry {
        path: rel.to_string(),
        data: None,
    });

    let mut children: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(iter) => iter
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .collect(),
        Err(err) => {
            log(format!(
                "clipboard tree: cannot list {}: {err}",
                dir.display()
            ));
            return;
        }
    };
    children.sort();

    for child in children {
        if entries.len() >= MAX_TREE_ENTRIES {
            log("clipboard tree: entry limit reached; sending a partial tree".to_string());
            return;
        }
        let meta = match fs::symlink_metadata(&child) {
            Ok(meta) => meta,
            Err(err) => {
                log(format!(
                    "clipboard tree: cannot stat {}: {err}",
                    child.display()
                ));
                continue;
            }
        };
        if meta.file_type().is_symlink() {
            log(format!("clipboard tree: skipping link {}", child.display()));
            continue;
        }
        let Some(name) = sanitized_component(&child) else {
            log(format!(
                "clipboard tree: skipping unusable name {}",
                child.display()
            ));
            continue;
        };
        let child_rel = format!("{rel}/{name}");
        if meta.is_dir() {
            if is_directory_link(&meta) {
                log(format!(
                    "clipboard tree: skipping directory link {}",
                    child.display()
                ));
                continue;
            }
            collect_dir(&child, &child_rel, depth + 1, entries, total, log);
        } else if meta.is_file() {
            let size = meta.len() as usize;
            if size > MAX_FILES_TOTAL || total.saturating_add(size) > MAX_FILES_TOTAL {
                log(format!(
                    "clipboard tree: skipping {} (size limit)",
                    child.display()
                ));
                continue;
            }
            match fs::read(&child) {
                Ok(data) => {
                    if total.saturating_add(data.len()) > MAX_FILES_TOTAL {
                        log(format!(
                            "clipboard tree: skipping {} (size limit)",
                            child.display()
                        ));
                        continue;
                    }
                    *total += data.len();
                    entries.push(WireEntry {
                        path: child_rel,
                        data: Some(data),
                    });
                }
                Err(err) => log(format!(
                    "clipboard tree: cannot read {}: {err}",
                    child.display()
                )),
            }
        }
    }
}

/// Junctions / mount points are directories with a reparse attribute, but
/// Rust does not report them as symlinks. Walking into one can duplicate or
/// loop a tree, so they are skipped. Cloud placeholder *files* are reparse
/// points too and must still be read, which is why this only applies to dirs.
fn is_directory_link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        false
    }
}

fn sanitized_component(path: &Path) -> Option<String> {
    let name = sanitize_name(path);
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn unique_root_name(path: &Path, used: &mut HashSet<String>) -> String {
    let base = sanitize_name(path);
    let base = if base.is_empty() {
        "item".to_string()
    } else {
        base
    };
    let mut name = base.clone();
    let mut index = 2;
    while !used.insert(name.to_lowercase()) {
        name = format!("{base} ({index})");
        index += 1;
    }
    name
}

fn apply_frame(app: &Arc<App>, watcher: &Arc<Mutex<ClipboardWatcher>>, bytes: &[u8]) -> Result<()> {
    let frame: Frame = postcard::from_bytes(bytes).context("decode clipboard frame")?;
    let summary = frame_summary(&frame);
    let fingerprint = frame_fingerprint(&frame);
    // Record before writing: from this moment the writer knows any change it
    // sees for this content is our own echo.
    *LAST_APPLIED.lock().unwrap_or_else(|e| e.into_inner()) = Some((fingerprint, Instant::now()));

    let mut file_dir: Option<PathBuf> = None;
    let content = match &frame {
        Frame::Ping => return Ok(()),
        Frame::Text(text) => ClipboardContent::Text(text.clone()),
        Frame::Image { width, height, png } => ClipboardContent::Image {
            width: *width,
            height: *height,
            png: png.clone(),
        },
        Frame::Files(files) => {
            let (paths, dir) = write_incoming_files(files)?;
            file_dir = Some(dir);
            ClipboardContent::Files(paths)
        }
        Frame::Rich { text, formats } => ClipboardContent::Rich {
            text: text.clone(),
            formats: formats
                .iter()
                .map(|format| lankvm_clipboard::NamedFormat {
                    name: format.name.clone(),
                    data: format.data.clone(),
                })
                .collect(),
        },
        Frame::Tree(entries) => {
            let (paths, dir) = write_incoming_tree(entries, &config_dir().join("clipboard"))?;
            file_dir = Some(dir);
            ClipboardContent::Files(paths)
        }
    };

    let mut last_err = None;
    for _ in 0..3 {
        // While this flag is set the writer will not consume the clipboard
        // sequence change caused by the write below.
        APPLYING.store(true, Ordering::SeqCst);
        let result = lankvm_clipboard::write_supported(&content);
        if result.is_ok() {
            SUPPRESSED_SEQ.store(lankvm_clipboard::sequence() as u64, Ordering::SeqCst);
        }
        APPLYING.store(false, Ordering::SeqCst);

        match result {
            Ok(()) => {
                watcher
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .acknowledge();
                match &file_dir {
                    Some(dir) => app.log(format!(
                        "clipboard received: {summary} -> {}",
                        dir.display()
                    )),
                    None => app.log(format!("clipboard received: {summary}")),
                }
                return Ok(());
            }
            Err(err) => {
                last_err = Some(err);
                thread::sleep(Duration::from_millis(80));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("write clipboard failed")))
}

fn recently_applied(fingerprint: u64) -> bool {
    let guard = LAST_APPLIED.lock().unwrap_or_else(|e| e.into_inner());
    match *guard {
        Some((fp, at)) => fp == fingerprint && at.elapsed() < REANNOUNCE_WINDOW,
        None => false,
    }
}

/// Identity of the content carried by a frame, computed from what is actually
/// sent (file bytes included), so two distinct files with the same name/size
/// are not confused.
fn frame_fingerprint(frame: &Frame) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match frame {
        Frame::Ping => 0u8.hash(&mut hasher),
        Frame::Text(text) => {
            1u8.hash(&mut hasher);
            text.hash(&mut hasher);
        }
        Frame::Image { width, height, png } => {
            2u8.hash(&mut hasher);
            width.hash(&mut hasher);
            height.hash(&mut hasher);
            png.hash(&mut hasher);
        }
        Frame::Files(files) => {
            3u8.hash(&mut hasher);
            for file in files {
                file.name.hash(&mut hasher);
                file.data.hash(&mut hasher);
            }
        }
        Frame::Rich { text, formats } => {
            4u8.hash(&mut hasher);
            text.hash(&mut hasher);
            for format in formats {
                format.name.hash(&mut hasher);
                format.data.hash(&mut hasher);
            }
        }
        Frame::Tree(entries) => {
            5u8.hash(&mut hasher);
            for entry in entries {
                entry.path.hash(&mut hasher);
                entry.data.hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

fn frame_summary(frame: &Frame) -> String {
    match frame {
        Frame::Ping => "ping".to_string(),
        Frame::Text(text) => format!("text ({} chars)", text.chars().count()),
        Frame::Image { width, height, png } => {
            format!("image {width}x{height} ({} bytes PNG)", png.len())
        }
        Frame::Files(files) => {
            let mut names: Vec<&str> = files.iter().take(3).map(|f| f.name.as_str()).collect();
            if files.len() > 3 {
                names.push("...");
            }
            format!("files ({}) [{}]", files.len(), names.join(", "))
        }
        Frame::Rich { text, formats } => {
            let names: Vec<&str> = formats.iter().map(|f| f.name.as_str()).collect();
            format!(
                "rich ({} chars) [{}]",
                text.chars().count(),
                names.join(", ")
            )
        }
        Frame::Tree(entries) => {
            let files = entries.iter().filter(|entry| entry.data.is_some()).count();
            let dirs = entries.len().saturating_sub(files);
            let bytes: usize = entries
                .iter()
                .filter_map(|entry| entry.data.as_ref())
                .map(|data| data.len())
                .sum();
            format!("tree ({files} files, {dirs} dirs, {bytes} bytes)")
        }
    }
}

fn write_incoming_files(files: &[WireFile]) -> Result<(Vec<PathBuf>, PathBuf)> {
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
    Ok((paths, dir))
}

fn write_incoming_tree(entries: &[WireEntry], root: &Path) -> Result<(Vec<PathBuf>, PathBuf)> {
    cleanup_clipboard_dirs(root);
    let dir = root.join(now_ms().to_string());
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;

    let mut roots: Vec<PathBuf> = Vec::new();
    for entry in entries.iter().take(MAX_TREE_ENTRIES) {
        let Some(relative) = safe_relative(&entry.path) else {
            continue;
        };
        let target = dir.join(&relative);
        if !target.starts_with(&dir) {
            continue;
        }
        match &entry.data {
            None => fs::create_dir_all(&target)
                .with_context(|| format!("create clipboard dir {}", target.display()))?,
            Some(data) => {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("create {}", parent.display()))?;
                }
                fs::write(&target, data)
                    .with_context(|| format!("write clipboard file {}", target.display()))?;
            }
        }
        if let Some(top) = relative.components().next() {
            let top_path = dir.join(top.as_os_str());
            if !roots.contains(&top_path) {
                roots.push(top_path);
            }
        }
    }

    if roots.is_empty() {
        bail!("empty clipboard tree");
    }
    Ok((roots, dir))
}

/// Turns a wire path into a relative path that cannot escape the bundle:
/// `..`, drive prefixes and absolute roots are rejected. Components are
/// sanitised again because the peer is not trusted to have done it.
fn safe_relative(path: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.ends_with(':') {
            return None;
        }
        let clean = sanitize_str(part);
        if clean.is_empty() {
            continue;
        }
        out.push(clean);
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_root(tag: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "lankvm-tree-{}-{}-{}",
            std::process::id(),
            tag,
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn scan_keeps_nested_shape_and_empty_dirs() {
        let src = temp_root("scan");
        fs::create_dir_all(src.join("album/2026")).unwrap();
        fs::create_dir_all(src.join("album/empty")).unwrap();
        fs::write(src.join("album/cover.txt"), b"cover").unwrap();
        fs::write(src.join("album/2026/a.txt"), b"a").unwrap();

        let mut messages = Vec::new();
        let mut log = |message: String| messages.push(message);
        let entries = build_tree_entries(&[src.join("album")], &mut log).unwrap();
        let paths: Vec<&str> = entries.iter().map(|entry| entry.path.as_str()).collect();

        assert!(paths.contains(&"album"));
        assert!(paths.contains(&"album/cover.txt"));
        assert!(paths.contains(&"album/2026"));
        assert!(paths.contains(&"album/2026/a.txt"));
        assert!(paths.contains(&"album/empty"));
        assert_eq!(
            entries.iter().filter(|entry| entry.data.is_some()).count(),
            2
        );
        assert!(messages.is_empty(), "{messages:?}");

        let _ = fs::remove_dir_all(&src);
    }

    #[test]
    fn same_basename_roots_are_disambiguated() {
        let src = temp_root("roots");
        fs::create_dir_all(src.join("a/album")).unwrap();
        fs::create_dir_all(src.join("b/album")).unwrap();
        fs::write(src.join("a/album/one.txt"), b"1").unwrap();
        fs::write(src.join("b/album/two.txt"), b"2").unwrap();

        let mut log = |_message: String| {};
        let entries =
            build_tree_entries(&[src.join("a/album"), src.join("b/album")], &mut log).unwrap();
        let paths: Vec<&str> = entries.iter().map(|entry| entry.path.as_str()).collect();

        assert!(paths.contains(&"album"));
        assert!(paths.contains(&"album (2)"));
        assert!(paths.contains(&"album/one.txt"));
        assert!(paths.contains(&"album (2)/two.txt"));
        let _ = fs::remove_dir_all(&src);
    }

    #[test]
    fn receive_writes_structure_and_rejects_traversal() {
        let base = temp_root("recv");
        let entries = vec![
            WireEntry {
                path: "album".into(),
                data: None,
            },
            WireEntry {
                path: "album/a.txt".into(),
                data: Some(b"a".to_vec()),
            },
            WireEntry {
                path: "album/sub".into(),
                data: None,
            },
            WireEntry {
                path: "album/sub/b.txt".into(),
                data: Some(b"b".to_vec()),
            },
            WireEntry {
                path: "../evil.txt".into(),
                data: Some(b"evil".to_vec()),
            },
            WireEntry {
                path: "C:/absolute.txt".into(),
                data: Some(b"absolute".to_vec()),
            },
        ];

        let (roots, dir) = write_incoming_tree(&entries, &base).unwrap();
        assert_eq!(roots.len(), 1);
        assert!(roots[0].ends_with("album"));
        assert_eq!(fs::read(dir.join("album/a.txt")).unwrap(), b"a");
        assert_eq!(fs::read(dir.join("album/sub/b.txt")).unwrap(), b"b");
        assert!(dir.join("album/sub").is_dir());
        assert!(!base.join("evil.txt").exists());
        assert!(!base.join("absolute.txt").exists());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn safe_relative_rejects_escape_attempts() {
        assert!(safe_relative("a/b.txt").is_some());
        assert!(safe_relative("a\\b.txt").is_some());
        assert!(safe_relative("..").is_none());
        assert!(safe_relative("a/../../b.txt").is_none());
        assert!(safe_relative("C:/Windows/evil.txt").is_none());
        assert!(safe_relative("/").is_none());
    }

    #[test]
    fn tree_frame_keeps_wire_tags_and_round_trips() {
        let frame = Frame::Tree(vec![
            WireEntry {
                path: "album".into(),
                data: None,
            },
            WireEntry {
                path: "album/a.txt".into(),
                data: Some(b"a".to_vec()),
            },
        ]);
        let bytes = postcard::to_allocvec(&frame).unwrap();
        // Tree is appended after the existing variants, so its tag is 5 and
        // older peers keep interpreting tags 0-4 exactly as before.
        assert_eq!(bytes[0], 5);

        let decoded: Frame = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(frame_fingerprint(&frame), frame_fingerprint(&decoded));
        assert_eq!(frame_summary(&frame), "tree (1 files, 1 dirs, 1 bytes)");
        assert_eq!(frame_summary(&decoded), frame_summary(&frame));
    }
}
