# LAN-KVM

![Platform](https://img.shields.io/badge/platform-Windows-0078D4?style=flat-square)
![Version](https://img.shields.io/badge/version-1.1.15-2ea44f?style=flat-square)
![UI](https://img.shields.io/badge/UI-English%20%2F%20%E7%B9%81%E9%AB%94%E4%B8%AD%E6%96%87-E67E22?style=flat-square)
![License](https://img.shields.io/badge/license-GPL--3.0-blue?style=flat-square)

**English** | [繁體中文](README.zh-TW.md)

Software KVM for Windows: share one keyboard and mouse across 2–3 computers on the same LAN.
Move the pointer past the screen edge, and the other machine takes over — including the
Windows sign-in screen and UAC prompts. Clipboard (text / image / files) is shared in both
directions, and the controlling machine's cursor disappears while the peer is in control.

> **Status: v1.1.15 — folder clipboard.** Verified on two Windows 11 machines on a
> wired LAN: edge switching, sign-in-screen control, Num Lock, boot auto-start, and bidirectional
> clipboard for text, tables, screenshots, files and folders.
>
> v1.1.15 lets a copied folder travel with its internal layout: subfolders (including empty
> ones) and relative paths are preserved, and the peer pastes the same tree instead of nothing.
> v1.1.14 carries rich clipboard formats (HTML, RTF and Excel XML) next to the plain text, so a
> range copied in Excel pastes as a formatted table — borders, fills, number formats and merged
> cells included. v1.1.13 fixed the return edge going unresponsive after operating on the peer
> for a while; the client now counts only the push at the entry edge. v1.1.12 moved the pointer
> before showing it and confirmed the return height; v1.1.10 made it return at the exit height
> and fixed edge switching after a display-mode change; v1.1.9 fixed the boot-time service crash.

---

## Features

- **Seamless edge switching** — push the pointer past the configured side; the peer takes over.
  Push back to the entry edge to return. Push distance, time window and return overshoot are adjustable.
- **Works at the sign-in screen and UAC** — the client side runs a LocalSystem service plus a
  `winsta0\Winlogon` helper process, so you can click the password field and type on a machine
  that is still logged out.
- **Bidirectional clipboard** — text, images, files, folders, and rich formats: a range copied in
  Excel (or content copied in Word) pastes with its table layout and styling intact, and a copied
  folder keeps its subfolders and relative paths. Plain-text targets still receive the text
  (TCP 45875, echo-suppressed, files land in `%LOCALAPPDATA%\LAN-KVM\clipboard\`).
- **Auto discovery and pairing** — LAN discovery, 3-digit pairing code, persistent IDs. Boot order
  and DHCP changes are handled automatically.
- **Auto start** — service starts at boot, tray app starts at sign-in, reconnects on its own.
- **Bilingual UI** — English by default, switchable to 繁體中文 in the app; the tray menu follows
  the same setting.
- **One-click diagnostics** — collect logs from both machines into one folder
  (last 2 MB per file, streamed). The UI shows current log sizes and has a **Clear logs** button.
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
LAN-KVM-Setup-1.1.15.exe
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
LAN-KVM-Setup-1.1.15.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
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
installer\build.cmd        # -> dist-app\LAN-KVM-Setup-1.1.15.exe
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
- [Latency and reconnect fixes (v1.1.1–1.1.7)](docs/15-latency-and-reconnect.md)
- [Release notes v1.1.7](docs/release-notes-v1.1.7.md)
- [Release notes v1.1.8](docs/release-notes-v1.1.8.md)
- [Release notes v1.1.9](docs/release-notes-v1.1.9.md)
- [Release notes v1.1.10](docs/release-notes-v1.1.10.md)
- [Release notes v1.1.11](docs/release-notes-v1.1.11.md)
- [Release notes v1.1.13](docs/release-notes-v1.1.13.md)
- [Release notes v1.1.14](docs/release-notes-v1.1.14.md)
- [Release notes v1.1.15](docs/release-notes-v1.1.15.md)
- [Log rotation and bundles](docs/16-log-retention.md)
- [AV / EDR and code signing strategy](docs/04-av-and-signing.md)

Most documents are written in Traditional Chinese.

## Known limitations

- **Unsigned binaries** — SmartScreen, McAfee or Apex One may warn or quarantine. Add a hash
  allow-list if needed; signing is planned.
- **Unencrypted LAN transport** — the P0 protocol is plain UDP/TCP and the pairing code has only
  3 digits. Use it on a trusted network only. QUIC/TLS + SPAKE2 is planned.
- **Windows only** — macOS/Linux are not implemented.
- **Clipboard needs both machines signed in** — the sign-in screen supports keyboard/mouse only.
- **Rich clipboard needs v1.1.14+ on both machines** — an older peer still syncs text, images and
  files, but cannot apply Excel/Word rich frames.
- **Folder copy needs v1.1.15+ on both machines** — older peers drop the tree frame; copy the
  folder's files instead. Folder copies are capped at 5,000 entries / 128 MB and skip directory
  junctions. File attributes, ACLs and timestamps are not preserved.
- **Multi-monitor / mixed DPI** is not verified yet.
- Tray icons on Windows 11 may be hidden behind `^`; drag it out once.

## License

[GPL-3.0-or-later](LICENSE). This is a clean-room implementation; no Deskflow/GPL-2.0-only code
was copied. If that changes, the license must be revisited.
