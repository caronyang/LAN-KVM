# LAN-KVM

軟體式 KVM：在同一個網段的 2–3 台電腦之間共用一組鍵盤/滑鼠，支援無縫邊界切換、剪貼簿共享與檔案傳輸。目標平台為 Windows / macOS / Linux，目前只有 Windows 可實測。

> 目前狀態：**v0.3-clip 測試版（跨機剪貼簿 + 控制端游標隱藏）**。v0.2 已完成兩台實機驗收（2026-10-23）：UDP 握手/邊界切換、Raw Input、絕對游標、Windows Service + SYSTEM session agent、`Default`/`Winlogon` 切換、Winlogon 滑鼠/鍵盤注入、單一安裝檔、系統匣、區網探索、3 位數配對、開機自動連線、瀏覽器 UI。**B 先開機停在登入頁時，A 可直接操作 B 登入**。v0.3 新增：任一台複製文字/圖片/檔案、另一台可貼上（TCP 45875），以及控制期間 master 游標真正消失（透明 overlay + 1×1 ClipCursor）。完整成果見 [`docs/12-stage-summary.md`](docs/12-stage-summary.md)、[`docs/13-clipboard-and-cursor.md`](docs/13-clipboard-and-cursor.md)。

## 文件

- [技術選型評估 v0.1](docs/01-tech-selection.md)
- [架構決策與修訂 v0.2](docs/02-decisions-and-architecture.md)
- [P0 PoC 使用說明](docs/03-p0-usage.md)
- [P0 兩台實機測試結果](docs/05-p0-results.md)
- [服務 + 登入畫面 PoC](docs/06-service-login-poc.md)
- [v0.1-local 單機驗證](docs/07-local-verification.md)
- [研發歷程與接手說明](docs/08-development-history.md)
- [新場域測試計畫（Gate 0–5）](docs/09-next-test-plan.md)
- [v0.2 安裝版 App（探索/配對/自動連線）](docs/11-app-installer.md)
- [v0.2 階段成果：兩台實機驗收完成](docs/12-stage-summary.md)
- [跨機剪貼簿與控制端游標隱藏（v0.3-clip）](docs/13-clipboard-and-cursor.md)
- [防毒／EDR 與程式簽章策略](docs/04-av-and-signing.md)
- [Logo 與品牌資產](assets/branding/gallery.html)（規範：`assets/branding/README.md`）

## 安裝版（v0.3-clip3）

```powershell
# 單一安裝檔；內嵌 App 與 SYSTEM 服務（跨機剪貼簿 + 游標隱藏）
dist-app\LAN-KVM-Setup-0.3-clip3.exe

# SHA-256
# 5D11F0CFED949EEC3FDA7DA85A4A5079FC65EC526A0437D2F2D5C1A1446274C1
```

> `-clip3` 修正**檔案貼上貼到舊檔**：`clipboard-win` 的檔案 setter 不會先清空剪貼簿，
> Explorer 舊的 `Shell IDList Array` 會蓋過新設的 CF_HDROP，貼上就變成上一個檔案。
> 現在寫入前會先 `EmptyClipboard`。
>
> `-clip2` 修正回音抑制：精確剪貼簿序號 + 3 秒內容窗，貼上後立刻複製的新內容不會被吞掉；
> log 會列出傳送的檔名與落地資料夾。
>
> v0.2 已驗收的登入頁版本保留為 `dist-app\LAN-KVM-Setup-0.2-login3.exe`
> （`8A622E5A9739429E6A77CD59916CB8FF57E7534C9690DEEF13C6C24B59D3EAD3`）。

安裝後常駐系統匣（Windows 11 可能在 `^` 裡，第一次拖到工作列固定）：

1. 左鍵匣圖示 → 開啟設定。
2. A 重新掃描 → 選 B → 配對。
3. B 會自動開 UI 顯示 3 位數；在 A 輸入。
4. 配對後自動連線；之後開機自動連，直到解除配對。
5. 右鍵選單：停止/重新連線、重新掃描、收集紀錄、結束。

詳細說明與限制見 [`docs/11-app-installer.md`](docs/11-app-installer.md)、[`dist-app/README.txt`](dist-app/README.txt)。

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

先確認 UDP 路徑（不需要 service，不需建置）：

```powershell
# B（管理員）
.\net-check.ps1 -Mode listen -Port 45871 -Seconds 120 -AddFirewallRule

# A
.\net-check.ps1 -Mode send -Peer <B_IP> -Port 45871
```

> 若 `.ps1` 被執行原則擋住，改用 `.\run-net-check.cmd …`，或先在該視窗跑 `Set-ExecutionPolicy -Scope Process Bypass -Force`。

看到 `RESULT: PASS - 10/10 replies` 再往下。

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
  lankvm-app/        # v0.2 常駐 App：探索/配對/UI/engine
  lankvm-setup/      # v0.2 安裝程式（防火牆/啟動/捷徑）
assets/branding/     # 定案 Logo（Seam）、ICO/ICNS/托盤圖示
dist/                # v0.1-local 測試包
docs/                # 技術評估、架構、測試與 AV 策略
```

## 目前方向

- 技術棧：Rust；v0.2 UI 是本機瀏覽器頁面，後續可換 Tauri 2 原生視窗
- 行程架構：常駐 App（UI + supervisor）+ 隱藏 engine 子程序；SYSTEM service 待整合
- 登入畫面：Windows 服務 + `winlogon` token + 每桌面 hook/injector；macOS/Linux 後續
- 傳輸：P0 用 UDP；正式版 QUIC（`quinn` + `rustls`）
- 探索/配對：mDNS-SD + SPAKE2 / SAS + 憑證 pinning（尚未實作）
- 游標：P0 相對注入；絕對定位已驗證，待接上協定
- 剪貼簿/檔案：本機引擎完成；跨機同步待傳輸層
- 品牌：LAN-KVM；Logo 定案 **Seam**

## 授權

**GPL-3.0-or-later**，完整條文見 [`LICENSE`](LICENSE)。目前是 clean-room 實作，不直接複製 Deskflow 的 GPL-2.0-only 程式碼；若之後決定移植，需改為 GPL-2.0-only 並審查相依授權。

## 下一步

1. ~~在乾淨 VM（無 Apex One）打通 A→B UDP，測跨機 Default/Winlogon 滑鼠+鍵盤。~~ **2026-10-23 新場域已完成**：UDP 10/10、Default/Winlogon 注入 `inject_fail=0`。
2. ~~v0.2 兩台實機驗收：安裝、探索、配對、開機自動連線、來回切換、解除配對、登入頁。~~ **2026-10-23 完成**，見 `docs/12-stage-summary.md`。
3. ~~把 SYSTEM service 接回 App，Winlogon/UAC 控制。~~ **已完成**（Winlogon helper process）。
4. QUIC/TLS + SPAKE2/憑證，取代 UDP + 3 位碼。
5. ~~跨機剪貼簿/檔案同步（本機引擎已完成，接傳輸層）。~~ **v0.3-clip 已接上 TCP 45875**，見 `docs/13-clipboard-and-cursor.md`。
6. 絕對定位游標接入協定（改善手感）。
7. 正式安裝器（Inno Setup/MSI）、程式與服務簽章、AV 白名單、正式卸載/升級。
8. 多螢幕/混合 DPI、24 小時 soak、斷線恢復。
9. macOS/Linux 平台層。
