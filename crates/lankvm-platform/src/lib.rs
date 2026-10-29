//! Platform abstraction and the Windows P0 implementation.
//!
//! The P0 binary is intentionally CLI-first: it proves input capture,
//! injection, desktop/session inspection, and later edge switching before a
//! GUI or a production service is added.

use lankvm_core::{MonitorInfo, Point, Rect};

pub mod logfile;

#[cfg(windows)]
pub mod windows;

#[cfg(windows)]
pub mod poc;

#[cfg(not(windows))]
pub mod windows {
    use super::*;

    pub fn enable_dpi_awareness() {}
    pub fn probe() -> anyhow::Result<ProbeReport> {
        anyhow::bail!("LAN-KVM P0 is currently Windows-only")
    }
    pub fn run_master(
        _peer: &str,
        _port: u16,
        _edge: lankvm_core::Edge,
        _seconds: u64,
        _trigger_px: f32,
        _trigger_ms: u64,
    ) -> anyhow::Result<()> {
        anyhow::bail!("LAN-KVM P0 is currently Windows-only")
    }
    pub fn run_client(_port: u16, _seconds: u64, _release_px: i32) -> anyhow::Result<()> {
        anyhow::bail!("LAN-KVM P0 is currently Windows-only")
    }
    pub fn run_hook_test(_seconds: u64, _suppress: bool) -> anyhow::Result<()> {
        anyhow::bail!("LAN-KVM P0 is currently Windows-only")
    }
    pub fn cursor_pos() -> anyhow::Result<Point> {
        anyhow::bail!("LAN-KVM P0 is currently Windows-only")
    }
    pub fn virtual_bounds() -> anyhow::Result<Rect> {
        anyhow::bail!("LAN-KVM P0 is currently Windows-only")
    }
    pub fn monitors() -> anyhow::Result<Vec<MonitorInfo>> {
        anyhow::bail!("LAN-KVM P0 is currently Windows-only")
    }
    pub fn active_desktop_name() -> anyhow::Result<String> {
        anyhow::bail!("LAN-KVM P0 is currently Windows-only")
    }
    pub fn move_relative(_dx: i32, _dy: i32) -> bool {
        false
    }
    pub fn move_absolute(_x: i32, _y: i32, _bounds: Rect) -> bool {
        false
    }
    pub fn mouse_button(_button: u8, _down: bool) -> bool {
        false
    }
    pub fn wheel(_delta: i32, _horizontal: bool) -> bool {
        false
    }
    pub fn key_scancode(_scan: u16, _down: bool, _extended: bool) -> bool {
        false
    }
    pub fn warp_cursor(_x: i32, _y: i32) -> bool {
        false
    }
    pub fn set_cursor_visible(_visible: bool) {}
    pub fn clip_to(_rect: Rect) {}
    pub fn release_clip() {}
    pub fn probe_report_display(_report: &ProbeReport) -> String {
        String::new()
    }
}

#[derive(Debug, Clone)]
pub struct ProbeReport {
    pub pid: u32,
    pub session_id: u32,
    pub active_console_session: u32,
    pub elevated: bool,
    pub integrity: String,
    pub desktop: String,
    pub desktop_error: Option<String>,
    pub monitors: Vec<MonitorInfo>,
    pub virtual_bounds: Rect,
    pub cursor: Point,
}

pub trait ScreenBackend {
    fn monitors(&self) -> anyhow::Result<Vec<MonitorInfo>>;
    fn virtual_bounds(&self) -> anyhow::Result<Rect>;
    fn cursor(&self) -> anyhow::Result<Point>;
}
