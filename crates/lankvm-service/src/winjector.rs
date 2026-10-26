//! Winlogon injector helper.
//!
//! This process is created by the session agent with
//! `lpDesktop = "winsta0\\Winlogon"`, so its main thread is born on the secure
//! desktop. That is what makes SendInput work where a thread attached to the
//! desktop from a Default-desktop process gets access denied.

use std::collections::HashSet;
use std::io::{self, Read};
use std::net::{TcpListener, TcpStream};

use anyhow::{Context, Result};
use lankvm_platform::windows as win;

use crate::logger::log;
use crate::remote_inject;

pub fn run(port: u16, parent_pid: u32) -> Result<()> {
    log(
        "session",
        format!("winjector: listening port={port} (process desktop should be Winlogon)"),
    );

    if parent_pid != 0 {
        std::thread::spawn(move || watch_parent(parent_pid));
    }

    let listener = TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("winjector bind 127.0.0.1:{port}"))?;

    loop {
        let (mut stream, _) = match listener.accept() {
            Ok(value) => value,
            Err(err) => {
                log("session", format!("winjector: accept failed: {err}"));
                break;
            }
        };
        let _ = stream.set_nodelay(true);
        log("session", "winjector: agent connected");
        process_connection(&mut stream);
        log("session", "winjector: agent disconnected");
    }

    log("session", "winjector: stopped");
    Ok(())
}

fn process_connection(stream: &mut TcpStream) {
    let mut pressed: HashSet<(u16, bool)> = HashSet::new();
    let mut frame = [0u8; 16];
    let mut failures = 0u32;
    let mut shift_down = false;

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
                let scan = a as u16;
                let down = b != 0;
                let is_shift = matches!(scan, 0x2A | 0x36);
                let shift = c & 0x100 != 0;
                if is_shift {
                    if down != shift_down {
                        let _ = win::key_scancode(0x2A, down, false);
                        shift_down = down;
                        if down {
                            pressed.insert((0x2A, false));
                        } else {
                            pressed.remove(&(0x2A, false));
                        }
                    }
                    true
                } else {
                    if shift != shift_down {
                        let _ = win::key_scancode(0x2A, shift, false);
                        shift_down = shift;
                        if shift {
                            pressed.insert((0x2A, false));
                        } else {
                            pressed.remove(&(0x2A, false));
                        }
                    }
                    let extended = c & 1 != 0;
                    let ok = win::key_scancode(scan, down, extended);
                    if down {
                        pressed.insert((scan, extended));
                    } else {
                        pressed.remove(&(scan, extended));
                    }
                    ok
                }
            }
            5 => win::wheel(a, b != 0),
            6 => {
                release_keys(&mut pressed);
                shift_down = false;
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
}

fn release_keys(pressed: &mut HashSet<(u16, bool)>) {
    for (scan, extended) in pressed.drain() {
        let _ = win::key_scancode(scan, false, extended);
    }
}

#[cfg(windows)]
fn watch_parent(parent_pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, INFINITE,
    };

    const SYNCHRONIZE: u32 = 0x0010_0000;

    unsafe {
        let handle = OpenProcess(SYNCHRONIZE, 0, parent_pid);
        if handle.is_null() {
            std::process::exit(0);
        }
        WaitForSingleObject(handle, INFINITE);
        CloseHandle(handle);
        std::process::exit(0);
    }
}
