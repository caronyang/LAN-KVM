//! Cross-thread injection channel.
//!
//! Injection runs on a dedicated thread that is attached to the target
//! desktop but does **not** install low-level hooks. Calling `SendInput` from
//! the hook thread itself is rejected by Windows in this setup, so the hook
//! worker forwards events to this injector thread.

use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy)]
pub enum InjectEvent {
    Switch {
        ratio: u16,
        entry_edge: i32,
    },
    Motion {
        dx: i32,
        dy: i32,
    },
    Button {
        button: u8,
        down: bool,
    },
    Key {
        scan: u16,
        down: bool,
        extended: bool,
    },
    Wheel {
        delta: i32,
        horizontal: bool,
    },
    Release,
}

fn sender_slot() -> &'static Mutex<Option<Sender<InjectEvent>>> {
    static SENDER: OnceLock<Mutex<Option<Sender<InjectEvent>>>> = OnceLock::new();
    SENDER.get_or_init(|| Mutex::new(None))
}

pub fn set_sender(sender: Sender<InjectEvent>) {
    if let Ok(mut slot) = sender_slot().lock() {
        *slot = Some(sender);
    }
}

pub fn clear_sender() {
    if let Ok(mut slot) = sender_slot().lock() {
        *slot = None;
    }
}

pub fn submit(event: InjectEvent) {
    if let Ok(slot) = sender_slot().lock() {
        if let Some(sender) = slot.as_ref() {
            let _ = sender.send(event);
        }
    }
}
