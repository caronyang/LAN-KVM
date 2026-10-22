# LAN-KVM

軟體式 KVM：在同一個網段的 2–3 台電腦之間共用一組鍵盤/滑鼠，支援無縫邊界切換、剪貼簿共享與檔案傳輸。目標平台為 Windows / macOS / Linux，目前只有 Windows 可實測。

> 目前狀態：**v0.1-local 單機可驗證版**。已驗證：UDP 握手/邊界切換、游標隱藏/釘住、Raw Input、絕對游標、剪貼簿文字/圖片/檔案、剪貼簿監聽、本機檔案分塊複製、Windows Service + SYSTEM session agent、`Default`/`Winlogon` 切換、Winlogon 滑鼠/鍵盤注入。跨機 UDP 目前被 Apex One/EDR 或 VM 網路阻擋，等 VM 環境放行。

## 文件

- [技術選型評估 v0.1](docs/01-tech-selection.md)
- [架構決策與修訂 v0.2](docs/02-decisions-and-architecture.md)
- [P0 PoC 使用說明](docs/03-p0-usage.md)
- [P0 兩台實機測試結果](docs/05-p0-results.md)
- [服務 + 登入畫面 PoC](docs/06-service-login-poc.md)
- [v0.1-local 單機驗證](docs/07-local-verification.md)
- [研發歷程與接手說明](docs/08-development-history.md)
- [防毒／EDR 與程式簽章策略](docs/04-av-and-signing.md)
- [Logo 與品牌資產](assets/branding/gallery.html)（規範：`assets/branding/README.md`）

## 快速開始（單機）

```powershell
# 先建置（Apex One 會刪 release/dev-fast service；用 debug）
cargo build -p lankvm-devtools -p lankvm-service -p lankvm-agent

# 一次跑完 probe / clipboard / file 測試
.\dist\run-local-tests.ps1
```

`abs-test` 請在滑鼠靜止、service 停止時手動跑：

```powershell
.\dist\lankvm-devtools.exe abs-test
```

Winlogon 本機測試：

```powershell
# B 的管理員 PowerShell
.\dist\install-service.ps1 -ClientPort 45871

# 執行後 15 秒內按 Win+L
.\dist\lankvm-devtools.exe local-winlogon-test --delay 15 --motion 300
```

詳細步驟與預期輸出見 [`docs/07-local-verification.md`](docs/07-local-verification.md)。

## 跨機測試（VM）

B：

```powershell
.\install-service.ps1 -ClientPort 45871
```

A：

```powershell
.\lankvm-devtools.exe poc --role master --peer <B_IP> --port 45871 --edge right --seconds 180
```

A 必須先看到 `[master] client online`，B 鎖定後等 `Winlogon: injector active`，再推邊界。

## 封裝

`dist/` 與 `LAN-KVM-v0.1-local.zip` 包含 debug 版 agent/devtools/service、安裝腳本、單機測試腳本與文件。

```
LAN-KVM-v0.1-local.zip
SHA256 CF57F02A0EE854DDDD746F0703888E2F7F9D32536A349B5770BAA151D9E96DCC
```

完整雜湊見 [`SHA256SUMS.txt`](SHA256SUMS.txt)。

> `lankvm-devtools.exe` 是開發工具（hook/injection/未加密 UDP），**不要散布**。正式產品是 `lankvm-agent.exe`；防毒/簽章策略見 [`docs/04-av-and-signing.md`](docs/04-av-and-signing.md)。若 devtools/service 被 AV 立即刪除，請用隔離 VM 或請 IT 對 SHA-256 加白名單，**不要關閉防毒**。

## Workspace

```
crates/
  lankvm-core/       # 幾何、佈局、共用型別
  lankvm-input/      # 邊界偵測、座標映射（純邏輯 + 測試）
  lankvm-net/        # P0 封包格式（正式版會換成 QUIC）
  lankvm-platform/   # Windows trait + probe/hook/injection/PoC
  lankvm-clipboard/  # 文字/圖片/CF_HDROP、監聽、postcard wire format
  lankvm-files/      # 分塊複製、BLAKE3、atomic rename
  lankvm-agent/      # 產品骨架：probe / version（不含開發工具）
  lankvm-devtools/   # 開發專用：local tests / poc（不散布）
  lankvm-service/    # Windows Service + SYSTEM session agent
assets/branding/     # 定案 Logo（Seam）、ICO/ICNS/托盤圖示
dist/                # v0.1-local 測試包
docs/                # 技術評估、架構、測試與 AV 策略
```

## 目前方向

- 技術棧：Rust + Tauri 2（Windows 優先，platform trait 預留 macOS/Linux）
- 行程架構：特權 supervisor service + SYSTEM session agent（Default/Winlogon）+ 一般權限 UI
- 登入畫面：Windows 服務 + `winlogon` token + 每桌面 hook/injector；macOS/Linux 後續
- 傳輸：P0 用 UDP；正式版 QUIC（`quinn` + `rustls`）
- 探索/配對：mDNS-SD + SPAKE2 / SAS + 憑證 pinning（尚未實作）
- 游標：P0 相對注入；絕對定位已驗證，待接上協定
- 剪貼簿/檔案：本機引擎完成；跨機同步待傳輸層
- 品牌：LAN-KVM；Logo 定案 **Seam**

## 授權

**GPL-3.0-or-later**，完整條文見 [`LICENSE`](LICENSE)。目前是 clean-room 實作，不直接複製 Deskflow 的 GPL-2.0-only 程式碼；若之後決定移植，需改為 GPL-2.0-only 並審查相依授權。

## 下一步

1. 在乾淨 VM（無 Apex One）打通 A→B UDP，測跨機 Default/Winlogon 滑鼠+鍵盤。
2. 接上 QUIC/TLS + mDNS/配對；把 P0 UDP 換掉。
3. 剪貼簿/檔案跨機同步（本機引擎已完成）。
4. 絕對定位游標接入 P0/v1（改善手感）。
5. Tauri UI、安裝檔、簽章、白名單。
