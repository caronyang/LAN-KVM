# LAN-KVM

![Platform](https://img.shields.io/badge/platform-Windows-0078D4?style=flat-square)
![Version](https://img.shields.io/badge/version-1.1.0-2ea44f?style=flat-square)
![UI](https://img.shields.io/badge/UI-English%20%2F%20%E7%B9%81%E9%AB%94%E4%B8%AD%E6%96%87-E67E22?style=flat-square)
![License](https://img.shields.io/badge/license-GPL--3.0-blue?style=flat-square)

[English](README.md) | **繁體中文**

Windows 軟體式 KVM：在同一個區域網路的 2–3 台電腦之間共用一組鍵盤／滑鼠。
把游標推過螢幕邊界，另一台就接手——包含 **Windows 登入畫面與 UAC**。
雙向剪貼簿（文字／圖片／檔案）與跨機檔案傳輸，控制期間原電腦的游標會消失。

> **狀態：v1.1.0（延遲與跳回修正）。** 已在兩台 Windows 11 實機驗收
> （同一 Wi-Fi 的筆電＋桌機）：邊界切換、登入畫面操作、Num Lock、
> 開機自動啟動、雙向剪貼簿（文字／截圖／檔案）。

---

## 功能

- **無縫邊界切換**：游標推過設定的邊界即切換；推回進入邊界即取回。
  切入推力、時間窗、回切越界都可調整。
- **登入畫面／UAC 可操作**：client 端由 LocalSystem 服務 + `winsta0\Winlogon`
  helper process 注入，即使對方還沒登入，也能點密碼欄位、輸入密碼。
- **雙向剪貼簿**：任一台複製文字／圖片／檔案，另一台直接貼上
  （TCP 45875、回音抑制；檔案會放在 `%LOCALAPPDATA%\LAN-KVM\clipboard\`）。
- **自動探索與配對**：區網探索、3 位數配對碼、持久化 ID；
  開機順序、DHCP 換 IP 都會自動處理。
- **開機自動啟動**：服務開機啟動、托盤程式登入後啟動、自動重連。
- **中英文介面**：預設 English，可在 UI 切換繁體中文；系統匣選單跟著同一個設定。
- **一鍵收集紀錄**：把兩台的 log 收進同一個資料夾。
- 沒有黑窗、不需要驅動、不需要額外硬體。

## 需求

| | |
|---|---|
| 作業系統 | Windows 10 / 11 x64 |
| 網路 | 兩台在同一個區域網路／網段 |
| 安裝 | 需要管理員權限（服務＋防火牆規則） |
| 剪貼簿 | 兩台都要登入 Windows（剪貼簿是使用者工作階段資源） |

## 安裝

從 **[Releases](https://github.com/caronyang/LAN-KVM/releases/latest)** 下載最新安裝檔：

```
LAN-KVM-Setup-1.1.0.exe
```

執行（UAC 按「是」），選 English 或 繁體中文，然後：

1. 在 **A** 開啟 LAN-KVM（系統匣圖示 → 開啟設定）。
2. 重新掃描 → 選 **B** → 配對。
3. **B** 會顯示 3 位數配對碼，在 **A** 輸入。
4. 完成。把游標推過 UI 設定的邊界即可控制。

安裝程式會建立服務、防火牆規則、開機排程、捷徑與登入自動啟動。
解除安裝請用 Windows「設定 → 應用程式」。

IT 大量部署可用無聲安裝：

```powershell
LAN-KVM-Setup-1.1.0.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
```

## 運作方式

| Port | Protocol | 用途 |
|------|----------|------|
| 45870 | UDP | 探索／配對 |
| 45871 | UDP | 控制（切換、釋放、keepalive） |
| 45872 | UDP | 輸入事件（滑鼠／鍵盤） |
| 45873 | TCP | 紀錄收集 |
| 45875 | TCP | 剪貼簿同步 |
| 45880 | TCP（loopback） | 瀏覽器 UI／本機 API |

- **Master** = 有實體鍵鼠的那台；**client** = 被控制的那台。
- client 的注入由 SYSTEM 服務執行，所以鎖定畫面也能用。
- 剪貼簿用簡單的長度前綴 + postcard 協定；client 監聽、master 主動連線。

## 從原始碼建置

需求：Rust 1.80+、Visual Studio Build Tools（MSVC）、Inno Setup 6.7+（只在打包安裝檔時需要）。

```powershell
cargo build --release -p lankvm-app -p lankvm-service
installer\build.cmd        # -> dist-app\LAN-KVM-Setup-1.1.0.exe
```

本機檢查：

```powershell
cargo test --workspace
cargo build -p lankvm-devtools
.\dist\run-local-tests.ps1
```

## 文件

- [架構決策與修訂 v0.2](docs/02-decisions-and-architecture.md)
- [P0 PoC 使用說明](docs/03-p0-usage.md)
- [P0 兩台實機測試結果](docs/05-p0-results.md)
- [服務 + 登入畫面 PoC](docs/06-service-login-poc.md)
- [v0.2 安裝版 App（探索／配對）](docs/11-app-installer.md)
- [兩台實機驗收總結](docs/12-stage-summary.md)
- [跨機剪貼簿與游標隱藏（v0.3）](docs/13-clipboard-and-cursor.md)
- [中英文介面與 Inno Setup 安裝程式（v0.4）](docs/14-bilingual-and-inno.md)
- [延遲與「跳回」修正（v1.1）](docs/15-latency-and-reconnect.md)
- [防毒／EDR 與程式簽章策略](docs/04-av-and-signing.md)

## 已知限制

- **未簽章**：SmartScreen／McAfee／Apex One 可能提示或隔離；必要時請加 hash 白名單，
  正式簽章已在規劃中。
- **P0 傳輸未加密**：純 UDP/TCP，配對碼只有 3 位數，只可在信任網段使用；
  下一版會換成 QUIC/TLS + SPAKE2。
- **僅支援 Windows**：macOS／Linux 尚未實作。
- **剪貼簿需要兩台都登入**：登入畫面只支援鍵鼠。
- **多螢幕／混合 DPI** 尚未實機驗收。
- Windows 11 的托盤圖示可能收在 `^` 裡，第一次請拖到工作列固定。

## 授權

[GPL-3.0-or-later](LICENSE)。本專案為 clean-room 實作，沒有複製 Deskflow／GPL-2.0-only
程式碼；若日後引用，授權需重新檢討。
