//! Winlogon injector helper.
//!
//! This process is created by the session agent with
//! `lpDesktop = "winsta0\\Winlogon"`, so its main thread is born on the secure
//! desktop. That is what makes SendInput work where a thread attached to the
//! desktop from a Default-desktop process gets access denied.

use std::collections::HashSet;
use std::io::{self, Read};
use std::net::TcpListener;

use anyhow::{Context, Result};
use lankvm_platform::windows as win;

use crate::logger::log;
use crate::remote_inject;

pub fn run(port: u16) -> Result<()> {
    log(
        "session",
        format!("winjector: listening port={port} (process desktop should be Winlogon)"),
    );

    let listener = TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("winjector bind 127.0.0.1:{port}"))?;
    let (mut stream, _) = listener.accept().context("winjector accept")?;
    let _ = stream.set_nodelay(true);
    log("session", "winjector: agent connected");

    let mut pressed: HashSet<(u16, bool)> = HashSet::new();
    let mut frame = [0u8; 16];
    let mut failures = 0u32;

    loop {
        match stream.read_exact(&mut frame) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(err) => {
                log("session", format!("winjector: read failed: {err}"));
                break;
            }
        }

        let (kind, a, b, c) = remote_inject::decode(&frame);
        let ok = match kind {
            1 => {
                // Switch: warp to the entry edge on this desktop.
                let entry_edge = b.clamp(1, 2);
                if let Ok(bounds) = win::virtual_bounds() {
                    let ratio = a.clamp(0, 65_535) as f32 / 65_535.0;
                    let y = bounds.top
                        + (ratio * (bounds.height().max(2) - 1) as f32).round() as i32;
                    let x = if entry_edge == 2 {
                        bounds.right - 1
                    } else {
                        bounds.left
                    };
                    let _ = win::warp_cursor(x, y);
                }
                true
            }
            2 => win::move_relative(a, b),
            3 => win::mouse_button(a as u8, b != 0),
            4 => {
                let ok = win::key_scancode(a as u16, b != 0, c != 0);
                if b != 0 {
                    pressed.insert((a as u16, c != 0));
                } else {
                    pressed.remove(&(a as u16, c != 0));
                }
                ok
            }
            5 => win::wheel(a, b != 0),
            6 => {
                release_keys(&mut pressed);
                true
            }
            _ => true,
        };

        if ok {
            failures = 0;
        } else {
            failures = failures.saturating_add(1);
            if failures == 1 || failures % 25 == 0 {
                log(
                    "session",
                    format!(
                        "winjector: SendInput failed kind={kind} streak={failures} err={}",
                        io::Error::last_os_error()
                    ),
                );
            }
        }
    }

    release_keys(&mut pressed);
    log("session", "winjector: stopped");
    Ok(())
}

fn release_keys(pressed: &mut HashSet<(u16, bool)>) {
    for (scan, extended) in pressed.drain() {
        let _ = win::key_scancode(scan, false, extended);
    }
}
