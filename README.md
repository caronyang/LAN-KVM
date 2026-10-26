# LAN-KVM

![Platform](https://img.shields.io/badge/platform-Windows-0078D4?style=flat-square)
![Version](https://img.shields.io/badge/version-1.1.3-2ea44f?style=flat-square)
![UI](https://img.shields.io/badge/UI-English%20%2F%20%E7%B9%81%E9%AB%94%E4%B8%AD%E6%96%87-E67E22?style=flat-square)
![License](https://img.shields.io/badge/license-GPL--3.0-blue?style=flat-square)

**English** | [繁體中文](README.zh-TW.md)

Software KVM for Windows: share one keyboard and mouse across 2–3 computers on the same LAN.
Move the pointer past the screen edge, and the other machine takes over — including the
Windows sign-in screen and UAC prompts. Clipboard (text / image / files) is shared in both
directions, and the controlling machine's cursor disappears while the peer is in control.

> **Status: v1.1.3 — latency, reconnect, keyboard and emergency-release fixes.** Verified on two Windows 11 machines
> (laptop + desktop on the same Wi-Fi): edge switching, sign-in-screen control,
> Num Lock, boot auto-start, and bidirectional clipboard for text, screenshots and files.
> v1.1 hardens the link against Wi-Fi stalls: 10 s liveness window, frame-coalesced mouse
> motion on both sides, debounced health probes, and IP-change handling that no longer
> drops a working session.

---

## Features

- **Seamless edge switching** — push the pointer past the configured side; the peer takes over.
  Push back to the entry edge to return. Push distance, time window and return overshoot are adjustable.
- **Works at the sign-in screen and UAC** — the client side runs a LocalSystem service plus a
  `winsta0\Winlogon` helper process, so you can click the password field and type on a machine
  that is still logged out.
- **Bidirectional clipboard** — copy text, images or files on either machine and paste on the other
  (TCP 45875, echo-suppressed, files land in `%LOCALAPPDATA%\LAN-KVM\clipboard\`).
- **Auto discovery and pairing** — LAN discovery, 3-digit pairing code, persistent IDs. Boot order
  and DHCP changes are handled automatically.
- **Auto start** — service starts at boot, tray app starts at sign-in, reconnects on its own.
- **Bilingual UI** — English by default, switchable to 繁體中文 in the app; the tray menu follows
  the same setting.
- **One-click diagnostics** — collect logs from both machines into one folder.
- **No black console windows**, no drivers, no extra hardware.

## Requirements

| | |
|---|---|
| OS | Windows 10 / 11 x64 |
| Network | Both machines on the same LAN / subnet |
| Install | Administrator rights (service + firewall rules) |
| Clipboard | Both machines must be signed in (clipboard is per user session) |

## Install

Download the latest installer from **[Releases](https://github.com/caronyang/LAN-KVM/releases/latest)**:

```
LAN-KVM-Setup-1.1.3.exe
```

Run it (UAC → Yes), choose English or 繁體中文, then:

1. Open LAN-KVM on machine **A** (tray icon → Open settings).
2. Rescan and pick machine **B** → **Pair**.
3. **B** shows a 3-digit code; type it on **A**.
4. Done — move the pointer past the screen edge configured in the UI.

The installer creates the service, firewall rules, boot tasks, shortcuts, and the sign-in
auto-start entry. Uninstall from Windows *Settings → Apps*.

Silent install for IT deployment:

```powershell
LAN-KVM-Setup-1.1.3.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
```

## If the mouse gets stuck

| Hotkey | What it does |
|---|---|
| `Ctrl+Alt+Shift+Q` | Normal release — use it whenever the engine is still responsive |
| `Ctrl+Alt+Shift+R` | **Force release** — ends the engine even if it is wedged, drops the cursor clip/overlay, and stops for 60 s |

The same action is in the tray menu (**Force release mouse**) and in the UI
(red button, `POST /api/emergency`). The hotkeys are global; they work while the
cursor is hidden and the screen is covered by the click-blocking overlay.

## How it works

| Port | Protocol | Purpose |
|------|----------|---------|
| 45870 | UDP | Discovery / pairing |
| 45871 | UDP | Control (switch, release, keepalive) |
| 45872 | UDP | Input events (mouse/keyboard) |
| 45873 | TCP | Log collection |
| 45875 | TCP | Clipboard sync |
| 45880 | TCP (loopback) | Browser UI / local API |

- **Master** = the machine with the physical keyboard/mouse; **client** = the machine being controlled.
- The client's input injection runs in a SYSTEM service so it survives the lock screen.
- Clipboard uses a small length-prefixed postcard protocol; the client listens and the master connects.

## Build from source

Prerequisites: Rust 1.80+, Visual Studio Build Tools (MSVC), Inno Setup 6.7+ (installer only).

```powershell
cargo build --release -p lankvm-app -p lankvm-service
installer\build.cmd        # -> dist-app\LAN-KVM-Setup-1.1.3.exe
```

Local checks:

```powershell
cargo test --workspace
cargo build -p lankvm-devtools
.\dist\run-local-tests.ps1
```

## Documentation

- [Architecture decisions (v0.2)](docs/02-decisions-and-architecture.md)
- [P0 PoC usage](docs/03-p0-usage.md)
- [Two-machine PoC results](docs/05-p0-results.md)
- [Service + sign-in screen PoC](docs/06-service-login-poc.md)
- [Installer, discovery and pairing (v0.2)](docs/11-app-installer.md)
- [Two-machine acceptance summary](docs/12-stage-summary.md)
- [Clipboard sync and cursor hiding (v0.3)](docs/13-clipboard-and-cursor.md)
- [Bilingual UI and Inno Setup installer (v0.4)](docs/14-bilingual-and-inno.md)
- [Latency and reconnect fixes (v1.1)](docs/15-latency-and-reconnect.md)
- [AV / EDR and code signing strategy](docs/04-av-and-signing.md)

Most documents are written in Traditional Chinese.

## Known limitations

- **Unsigned binaries** — SmartScreen, McAfee or Apex One may warn or quarantine. Add a hash
  allow-list if needed; signing is planned.
- **Unencrypted LAN transport** — the P0 protocol is plain UDP/TCP and the pairing code has only
  3 digits. Use it on a trusted network only. QUIC/TLS + SPAKE2 is planned.
- **Windows only** — macOS/Linux are not implemented.
- **Clipboard needs both machines signed in** — the sign-in screen supports keyboard/mouse only.
- **Multi-monitor / mixed DPI** is not verified yet.
- Tray icons on Windows 11 may be hidden behind `^`; drag it out once.

## License

[GPL-3.0-or-later](LICENSE). This is a clean-room implementation; no Deskflow/GPL-2.0-only code
was copied. If that changes, the license must be revisited.
