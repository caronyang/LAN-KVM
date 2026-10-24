# LAN-KVM 1.0.0

First public release. Windows 10 / 11 x64.

## Download

| File | Size | SHA-256 |
|---|---|---|
| `LAN-KVM-Setup-1.0.0.exe` | 2.96 MB | `C399AE8ABC1B810C602E2933F3174A8F5136376C16FBF22B7913169B648B27E0` |

## Highlights

- **Edge switching** between 2–3 machines on the same LAN, with adjustable push distance,
  time window and return overshoot.
- **Sign-in screen / UAC control** — input injection runs in a LocalSystem service with a
  `winsta0\Winlogon` helper, so the peer can be controlled before anyone logs in.
- **Bidirectional clipboard** for text, images and files (TCP 45875).
- **Cursor hiding on the controlling machine** while the peer has control.
- **Auto discovery + 3-digit pairing**, persistent IDs, DHCP changes and boot order handled.
- **Boot auto-start** (service + tray), auto reconnect, one-click log collection.
- **Bilingual UI** — English by default, switch to 繁體中文 in the app; tray follows the same setting.
- **Inno Setup installer** — standard Apps & Features entry, upgrade and uninstall.

## Install

1. Run `LAN-KVM-Setup-1.0.0.exe` (UAC → Yes) on both machines.
2. Machine A: tray → Open settings → Rescan → select B → Pair.
3. B shows a 3-digit code; type it on A.
4. Push the pointer past the configured screen edge.

Silent install:

```powershell
LAN-KVM-Setup-1.0.0.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
```

## Known limitations

- Binaries are **unsigned**; SmartScreen or endpoint security may warn.
- The LAN transport is **unencrypted** and pairing uses a 3-digit code — trusted networks only.
- **Windows only**; multi-monitor / mixed DPI is not verified yet.
- Clipboard sync needs both machines **signed in** (the sign-in screen supports keyboard/mouse only).

---

# 繁體中文

首個公開發佈版，Windows 10 / 11 x64。

## 下載

| 檔案 | 大小 | SHA-256 |
|---|---|---|
| `LAN-KVM-Setup-1.0.0.exe` | 2.96 MB | `C399AE8ABC1B810C602E2933F3174A8F5136376C16FBF22B7913169B648B27E0` |

## 重點功能

- **邊界切換**：同網段 2–3 台共用一組鍵鼠，切入推力／時間窗／回切越界可調。
- **登入畫面／UAC**：client 由 LocalSystem 服務 + `winsta0\Winlogon` helper 注入，
  對方還沒登入也能操作。
- **雙向剪貼簿**：文字／圖片／檔案（TCP 45875）。
- **控制端游標隱藏**：控制對方時原電腦游標消失，回切立即恢復。
- **自動探索 + 3 位數配對**、持久化 ID、換 IP 與開機順序自動處理。
- **開機自動啟動**（服務＋托盤）、自動重連、一鍵收集兩台紀錄。
- **中英文介面**：預設 English，可在 UI 切換繁體中文；系統匣同步。
- **Inno Setup 安裝程式**：標準「應用程式與功能」項目、可升級、可解除安裝。

## 安裝

1. 兩台都執行 `LAN-KVM-Setup-1.0.0.exe`（UAC 按是）。
2. A：系統匣 → 開啟設定 → 重新掃描 → 選 B → 配對。
3. B 顯示 3 位數配對碼，在 A 輸入。
4. 把游標推過設定的邊界即可控制。

無聲安裝：

```powershell
LAN-KVM-Setup-1.0.0.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
```

## 已知限制

- 執行檔**未簽章**，SmartScreen 或端點防護可能提示。
- 區網傳輸**未加密**、配對碼只有 3 位數，只可在信任網段使用。
- **僅支援 Windows**；多螢幕／混合 DPI 尚未驗收。
- 剪貼簿需要兩台都**已登入**（登入畫面只支援鍵鼠）。
