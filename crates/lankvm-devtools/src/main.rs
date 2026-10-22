//! Development-only tools for the LAN-KVM P0 prototype.
//!
//! This binary installs global hooks, injects input, and sends unauthenticated
//! UDP. It is **not** part of the shipped product and must not be distributed
//! to end users. The product binary is `lankvm-agent`.

use std::env;
use std::process::ExitCode;

use anyhow::{anyhow, Result};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("probe") => {
            let report = lankvm_platform::windows::probe()?;
            print_report(&report);
            Ok(())
        }
        #[cfg(windows)]
        Some("inject-test") => inject_test(),
        #[cfg(not(windows))]
        Some("inject-test") => Err(anyhow!("LAN-KVM P0 is currently Windows-only")),
        Some("hook-test") => hook_test(args),
        #[cfg(windows)]
        Some("abs-test") => abs_test(),
        #[cfg(not(windows))]
        Some("abs-test") => Err(anyhow!("LAN-KVM P0 is currently Windows-only")),
        #[cfg(windows)]
        Some("clipboard-test") => clipboard_test(args),
        #[cfg(not(windows))]
        Some("clipboard-test") => Err(anyhow!("LAN-KVM P0 is currently Windows-only")),
        #[cfg(windows)]
        Some("clipboard-watch") => clipboard_watch(args),
        #[cfg(not(windows))]
        Some("clipboard-watch") => Err(anyhow!("LAN-KVM P0 is currently Windows-only")),
        Some("file-test") => file_test(args),
        #[cfg(windows)]
        Some("local-winlogon-test") => local_winlogon_test(args),
        #[cfg(not(windows))]
        Some("local-winlogon-test") => Err(anyhow!("LAN-KVM P0 is currently Windows-only")),
        #[cfg(windows)]
        Some("poc") => run_poc(args),
        #[cfg(not(windows))]
        Some("poc") => Err(anyhow!("LAN-KVM P0 is currently Windows-only")),
        Some("help") | Some("-h") | Some("--help") | None => {
            usage();
            Ok(())
        }
        Some(other) => Err(anyhow!("unknown command: {other}")),
    }
}

fn hook_test(mut args: impl Iterator<Item = String>) -> Result<()> {
    let mut seconds = 10u64;
    let mut suppress = false;
    let mut confirmed = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seconds" => {
                seconds = args
                    .next()
                    .ok_or_else(|| anyhow!("--seconds needs a value"))?
                    .parse()?;
            }
            "--suppress" => suppress = true,
            "--yes" => confirmed = true,
            other => return Err(anyhow!("unknown argument: {other}")),
        }
    }
    if suppress && !confirmed {
        return Err(anyhow!(
            "--suppress blocks keyboard/buttons; add --yes to confirm"
        ));
    }
    lankvm_platform::windows::run_hook_test(seconds, suppress)
}

#[cfg(windows)]
fn inject_test() -> Result<()> {
    let before = lankvm_platform::windows::cursor_pos()?;
    let bounds = lankvm_platform::windows::virtual_bounds()?;
    let dx = if before.x < bounds.right - 40 {
        20
    } else {
        -20
    };
    if !lankvm_platform::windows::move_relative(dx, 0) {
        return Err(anyhow!("SendInput failed"));
    }
    std::thread::sleep(std::time::Duration::from_millis(120));
    let after = lankvm_platform::windows::cursor_pos()?;
    let _ = lankvm_platform::windows::move_relative(-dx, 0);
    println!(
        "cursor: ({}, {}) -> ({}, {}), expected x {:+}",
        before.x, before.y, after.x, after.y, dx
    );
    if after.x == before.x {
        return Err(anyhow!("cursor did not move; SendInput may be blocked"));
    }
    Ok(())
}

#[cfg(windows)]
fn run_poc(mut args: impl Iterator<Item = String>) -> Result<()> {
    use lankvm_core::Edge;

    let mut role: Option<String> = None;
    let mut peer: Option<String> = None;
    let mut port = lankvm_platform::poc::DEFAULT_PORT;
    let mut edge = Edge::Right;
    let mut seconds = 0u64;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--role" => {
                role = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--role needs master or client"))?,
                );
            }
            "--peer" => {
                peer = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--peer needs an IP/hostname"))?,
                );
            }
            "--port" => {
                port = args
                    .next()
                    .ok_or_else(|| anyhow!("--port needs a value"))?
                    .parse()?;
            }
            "--edge" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("--edge needs right or left"))?;
                edge = match value.as_str() {
                    "right" => Edge::Right,
                    "left" => Edge::Left,
                    _ => return Err(anyhow!("--edge must be right or left")),
                };
            }
            "--seconds" => {
                seconds = args
                    .next()
                    .ok_or_else(|| anyhow!("--seconds needs a value"))?
                    .parse()?;
            }
            other => return Err(anyhow!("unknown argument: {other}")),
        }
    }

    match role.as_deref() {
        Some("master") => {
            let peer = peer.ok_or_else(|| anyhow!("master requires --peer <client-ip>"))?;
            lankvm_platform::poc::run_master(&peer, port, edge, seconds)
        }
        Some("client") => lankvm_platform::poc::run_client(port, seconds),
        _ => Err(anyhow!("--role must be master or client")),
    }
}

#[cfg(windows)]
fn abs_test() -> Result<()> {
    use std::thread::sleep;
    use std::time::Duration;

    let bounds = lankvm_platform::windows::virtual_bounds()?;
    let original = lankvm_platform::windows::cursor_pos()?;
    let center = bounds.center();
    let points = [
        center,
        lankvm_core::Point::new(center.x + 120, center.y),
        lankvm_core::Point::new(center.x - 120, center.y),
    ];

    let mut failed = false;
    for point in points {
        let mut passed = false;
        for attempt in 1..=3 {
            if !lankvm_platform::windows::move_absolute(point.x, point.y, bounds) {
                return Err(anyhow!("absolute SendInput failed at {point:?}"));
            }
            sleep(Duration::from_millis(80));
            let got = lankvm_platform::windows::cursor_pos()?;
            let dx = (got.x - point.x).abs();
            let dy = (got.y - point.y).abs();
            println!(
                "target=({}, {}) got=({}, {}) delta=({dx}, {dy}) attempt={attempt}",
                point.x, point.y, got.x, got.y
            );
            if dx <= 3 && dy <= 3 {
                passed = true;
                break;
            }
        }
        if !passed {
            failed = true;
            println!("WARN: cursor moved during the test (physical mouse?) — rerun when idle");
        }
    }

    let _ = lankvm_platform::windows::warp_cursor(original.x, original.y);
    if failed {
        return Err(anyhow!("absolute positioning error too large"));
    }
    println!("absolute cursor test PASS");
    Ok(())
}

#[cfg(windows)]
fn clipboard_test(mut args: impl Iterator<Item = String>) -> Result<()> {
    use lankvm_clipboard as clip;
    use std::thread::sleep;
    use std::time::Duration;

    let mut files: Vec<std::path::PathBuf> = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--files" => {
                for path in args.by_ref() {
                    files.push(path.into());
                }
            }
            other => return Err(anyhow!("unknown clipboard-test argument: {other}")),
        }
    }

    let original = clip::read_supported().ok();
    let text = format!("LAN-KVM clipboard test {}", std::process::id());
    clip::write_text(&text)?;
    sleep(Duration::from_millis(150));
    let got = clip::read_text()?;
    if got != text {
        return Err(anyhow!("text clipboard round-trip failed"));
    }
    let wire = clip::encode(&lankvm_clipboard::ClipboardContent::Text(got.clone()))?;
    let decoded = clip::decode(&wire)?;
    if decoded != lankvm_clipboard::ClipboardContent::Text(got.clone()) {
        return Err(anyhow!("text wire round-trip failed"));
    }
    println!("text PASS: {text} (wire {} bytes)", wire.len());

    let width = 16u32;
    let height = 16u32;
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            rgba.extend_from_slice(&[(x * 16) as u8, (y * 16) as u8, 64, 255]);
        }
    }
    let png = clip::encode_png(width, height, &rgba)?;
    clip::write_image_png(&png)?;
    sleep(Duration::from_millis(150));
    let (got_w, got_h, got_png) = clip::read_image_png()?;
    if (got_w, got_h) != (width, height) {
        return Err(anyhow!(
            "image dimensions changed: got {got_w}x{got_h}, expected {width}x{height}"
        ));
    }
    let (_, _, decoded) = clip::decode_png(&got_png)?;
    if decoded.len() != rgba.len() {
        return Err(anyhow!("image pixel buffer size changed"));
    }
    let wire = clip::encode(&lankvm_clipboard::ClipboardContent::Image {
        width: got_w,
        height: got_h,
        png: got_png,
    })?;
    let decoded = clip::decode(&wire)?;
    if let lankvm_clipboard::ClipboardContent::Image { width, height, .. } = decoded {
        if (width, height) != (got_w, got_h) {
            return Err(anyhow!("image wire round-trip changed dimensions"));
        }
    } else {
        return Err(anyhow!("image wire round-trip changed type"));
    }
    println!("image PASS: {got_w}x{got_h} (wire {} bytes)", wire.len());

    if !files.is_empty() {
        for path in &files {
            if !path.exists() {
                return Err(anyhow!("file does not exist: {}", path.display()));
            }
        }
        clip::write_files(&files)?;
        sleep(Duration::from_millis(150));
        let got_files = clip::read_files()?;
        if got_files.len() != files.len() {
            return Err(anyhow!(
                "file clipboard count changed: got {}, expected {}",
                got_files.len(),
                files.len()
            ));
        }
        let wire = clip::encode(&lankvm_clipboard::ClipboardContent::Files(
            got_files.clone(),
        ))?;
        match clip::decode(&wire)? {
            lankvm_clipboard::ClipboardContent::Files(decoded) if decoded == got_files => {}
            _ => return Err(anyhow!("files wire round-trip failed")),
        }
        println!(
            "files PASS: {} entries (wire {} bytes)",
            got_files.len(),
            wire.len()
        );
    } else {
        println!("files SKIP: pass --files <path> to test CF_HDROP");
    }

    if let Some(original) = original {
        let _ = clip::write_supported(&original);
    } else {
        let _ = clip::write_text("");
    }
    println!("clipboard test PASS");
    Ok(())
}

#[cfg(windows)]
fn clipboard_watch(mut args: impl Iterator<Item = String>) -> Result<()> {
    use lankvm_clipboard::{content_summary, ClipboardWatcher};
    use std::thread::sleep;
    use std::time::{Duration, Instant};

    let mut seconds = 15u64;
    let mut verbose = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seconds" => {
                seconds = args
                    .next()
                    .ok_or_else(|| anyhow!("--seconds needs a value"))?
                    .parse()?;
            }
            "--verbose" => verbose = true,
            other => return Err(anyhow!("unknown clipboard-watch argument: {other}")),
        }
    }

    let mut watcher = ClipboardWatcher::new();
    let started = Instant::now();
    println!("Watching clipboard for {seconds}s; copy text/image/files now.");
    while started.elapsed() < Duration::from_secs(seconds) {
        if let Some(content) = watcher.poll() {
            println!("[clipboard] {}", content_summary(&content));
            if verbose {
                if let lankvm_clipboard::ClipboardContent::Text(text) = content {
                    println!("  text: {text}");
                }
            }
        }
        sleep(Duration::from_millis(200));
    }
    println!("clipboard watch done");
    Ok(())
}

fn file_test(mut args: impl Iterator<Item = String>) -> Result<()> {
    use lankvm_files::{copy_files, hash_file, CopyOptions};

    let mut source: Option<std::path::PathBuf> = None;
    let mut dest: Option<std::path::PathBuf> = None;
    let mut overwrite = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--source" => {
                source = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--source needs a path"))?
                        .into(),
                );
            }
            "--dest" => {
                dest = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--dest needs a directory"))?
                        .into(),
                );
            }
            "--overwrite" => overwrite = true,
            other => return Err(anyhow!("unknown file-test argument: {other}")),
        }
    }

    let source = source.ok_or_else(|| anyhow!("file-test requires --source"))?;
    let dest = dest.ok_or_else(|| anyhow!("file-test requires --dest"))?;
    let options = CopyOptions {
        chunk_size: 64 * 1024,
        overwrite,
    };
    let report = copy_files(&[source], &dest, &options)?;
    for file in &report.files {
        let actual = hash_file(&file.destination)?;
        if actual != file.blake3 {
            return Err(anyhow!("hash mismatch for {}", file.destination.display()));
        }
        println!(
            "PASS {} -> {} ({} bytes, {})",
            file.source.display(),
            file.destination.display(),
            file.bytes,
            file.blake3
        );
    }
    println!("total bytes: {}", report.total_bytes);
    Ok(())
}

#[cfg(windows)]
fn local_winlogon_test(mut args: impl Iterator<Item = String>) -> Result<()> {
    use lankvm_net::{Packet, PacketKind};
    use std::net::UdpSocket;
    use std::thread::sleep;
    use std::time::Duration;

    let mut port = 45_871u16;
    let mut delay = 10u64;
    let mut motion = 300i32;
    let mut send_key = true;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                port = args
                    .next()
                    .ok_or_else(|| anyhow!("--port needs a value"))?
                    .parse()?;
            }
            "--delay" => {
                delay = args
                    .next()
                    .ok_or_else(|| anyhow!("--delay needs a value"))?
                    .parse()?;
            }
            "--motion" => {
                motion = args
                    .next()
                    .ok_or_else(|| anyhow!("--motion needs a value"))?
                    .parse()?;
            }
            "--no-key" => send_key = false,
            other => return Err(anyhow!("unknown local-winlogon-test argument: {other}")),
        }
    }

    let socket = UdpSocket::bind(("127.0.0.1", 0))?;
    let target = ("127.0.0.1", port);
    let mut seq = 0u32;
    let mut send = |kind: PacketKind, a: i32, b: i32| -> Result<()> {
        let mut packet = Packet::new(kind, seq);
        seq = seq.wrapping_add(1);
        packet.a = a;
        packet.b = b;
        socket.send_to(&packet.encode(), target)?;
        Ok(())
    };

    println!(
        "Sending Switch/Motion{} to 127.0.0.1:{port} in {delay}s.",
        if send_key { "/CapsLock" } else { "" }
    );
    println!("Lock the screen now (Win+L).");
    sleep(Duration::from_secs(delay));

    send(PacketKind::Hello, 0, 0)?;
    send(PacketKind::Switch, 32_767, 1)?;
    send(PacketKind::Motion, motion, 0)?;
    if send_key {
        send(PacketKind::Key, 0x3A, 1)?; // Caps Lock down
        send(PacketKind::Key, 0x3A, 0)?; // Caps Lock up
    }
    println!("sent; check the cursor and Caps Lock LED");
    Ok(())
}

fn usage() {
    println!(
        r#"LAN-KVM devtools (P0) — DEVELOPMENT ONLY, NOT FOR DISTRIBUTION

Usage:
  lankvm-devtools probe                 Print session, desktop, DPI, monitors, cursor
  lankvm-devtools inject-test           Move the cursor 20 px and back (SendInput check)
  lankvm-devtools abs-test              Move the cursor with absolute coordinates
  lankvm-devtools clipboard-test [--files <path>...]  Clipboard text/image/file round-trip
  lankvm-devtools clipboard-watch [--seconds N]  Watch clipboard changes (metadata only)
  lankvm-devtools file-test --source <file> --dest <dir> [--overwrite]
  lankvm-devtools local-winlogon-test [--port N] [--delay N]  Send P0 packets to localhost
  lankvm-devtools hook-test [options]   Install low-level hooks and log events
  lankvm-devtools poc --role master --peer <ip> [options]
  lankvm-devtools poc --role client [options]

hook-test options:
  --seconds N   Stop after N seconds (default 10; 0 = until Ctrl+C)
  --suppress    Block keyboard and mouse buttons while running
  --yes         Required with --suppress

poc options:
  --role master|client   Required
  --peer <ip>            Client IP (master only)
  --port N               UDP port (default 45871)
  --edge right|left      Master exit edge (default right)
  --seconds N            Stop after N seconds (default 0 = until Ctrl+C)

WARNING: this tool is unencrypted, unauthenticated, and logs key events.
It is intended only for a dedicated test machine. Do not ship it.

The product binary is lankvm-agent; it has no hook/injection/keylogging code."#
    );
}

fn print_report(report: &lankvm_platform::ProbeReport) {
    println!("LAN-KVM P0 probe");
    println!("  pid: {}", report.pid);
    println!(
        "  session: {} (active console session: {})",
        report.session_id,
        if report.active_console_session == u32::MAX {
            "none".to_string()
        } else {
            report.active_console_session.to_string()
        }
    );
    println!(
        "  elevation: {} / integrity: {}",
        report.elevated, report.integrity
    );
    match &report.desktop_error {
        None => println!("  input desktop: {}", report.desktop),
        Some(err) => println!("  input desktop: {} ({})", report.desktop, err),
    }
    println!(
        "  virtual bounds: {}x{}+{},{}",
        report.virtual_bounds.width(),
        report.virtual_bounds.height(),
        report.virtual_bounds.left,
        report.virtual_bounds.top
    );
    println!("  cursor: ({}, {})", report.cursor.x, report.cursor.y);
    println!("  monitors:");
    for monitor in &report.monitors {
        println!(
            "    #{} {} {}x{}+{},{} dpi={}{}",
            monitor.id,
            if monitor.name.is_empty() {
                "<unnamed>"
            } else {
                &monitor.name
            },
            monitor.rect.width(),
            monitor.rect.height(),
            monitor.rect.left,
            monitor.rect.top,
            monitor.dpi,
            if monitor.primary { " primary" } else { "" }
        );
    }
}
