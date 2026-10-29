//! Route injection to the per-desktop helper process.
//!
//! The session agent lives on the Default desktop. On Winlogon a separate
//! helper process is created with `lpDesktop = winsta0\Winlogon`; it owns a
//! thread whose process desktop is the secure desktop and performs SendInput
//! there. Events are sent over a loopback TCP connection.

use std::io::Write;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::inject::InjectEvent;

static STREAM: Mutex<Option<TcpStream>> = Mutex::new(None);
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Keep the connection open but do not route input yet.
pub fn attach(stream: TcpStream) {
    if let Ok(mut slot) = STREAM.lock() {
        *slot = Some(stream);
    }
    ENABLED.store(false, Ordering::SeqCst);
}

pub fn enable() {
    ENABLED.store(true, Ordering::SeqCst);
}

pub fn disable() {
    ENABLED.store(false, Ordering::SeqCst);
}

pub fn clear() {
    ENABLED.store(false, Ordering::SeqCst);
    if let Ok(mut slot) = STREAM.lock() {
        *slot = None;
    }
}

pub fn is_active() -> bool {
    ENABLED.load(Ordering::SeqCst) && STREAM.lock().map(|slot| slot.is_some()).unwrap_or(false)
}

pub fn connected() -> bool {
    STREAM.lock().map(|slot| slot.is_some()).unwrap_or(false)
}

pub fn send(event: &InjectEvent) -> bool {
    if !ENABLED.load(Ordering::SeqCst) {
        return false;
    }
    let Ok(mut guard) = STREAM.lock() else {
        return false;
    };
    let Some(stream) = guard.as_mut() else {
        return false;
    };
    let frame = encode(event);
    if stream.write_all(&frame).is_ok() {
        true
    } else {
        *guard = None;
        ENABLED.store(false, Ordering::SeqCst);
        false
    }
}

fn encode(event: &InjectEvent) -> [u8; 16] {
    let mut frame = [0u8; 16];
    let (kind, a, b, c) = match *event {
        InjectEvent::Switch { ratio, entry_edge } => (1u8, ratio as i32, entry_edge, 0),
        InjectEvent::Motion { dx, dy } => (2u8, dx, dy, 0),
        InjectEvent::Button { button, down } => (3u8, button as i32, down as i32, 0),
        InjectEvent::Key {
            scan,
            down,
            extended,
            shift,
            shift_known,
        } => (
            4u8,
            scan as i32,
            down as i32,
            (extended as i32) | ((shift as i32) << 8) | ((shift_known as i32) << 9),
        ),
        InjectEvent::Wheel { delta, horizontal } => (5u8, delta, horizontal as i32, 0),
        InjectEvent::Release => (6u8, 0, 0, 0),
    };
    frame[0] = kind;
    frame[4..8].copy_from_slice(&a.to_le_bytes());
    frame[8..12].copy_from_slice(&b.to_le_bytes());
    frame[12..16].copy_from_slice(&c.to_le_bytes());
    frame
}

pub fn decode(frame: &[u8; 16]) -> (u8, i32, i32, i32) {
    let a = i32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]);
    let b = i32::from_le_bytes([frame[8], frame[9], frame[10], frame[11]]);
    let c = i32::from_le_bytes([frame[12], frame[13], frame[14], frame[15]]);
    (frame[0], a, b, c)
}

pub const PORT: u16 = 45_874;
