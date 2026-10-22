# LAN-KVM 研發歷程與接手說明

> 日期：2026-10-22  
> 版本：v0.1-local  
> Repository：<https://github.com/caronyang/LAN-KVM>（Private）  
> 初始快照：`2d4b6c5 Initial commit: LAN-KVM v0.1-local`

這份文件記錄專案從需求、技術選型、PoC、失敗與修正，到目前可交接的狀態。目標是讓另一台電腦／另一位開發者不用重讀整段對話，也能在 30 分鐘內接手。

---

## 1. 專案目標

做一套常駐式軟體 KVM：

- Windows / macOS / Linux 架構，但目前只有 Windows 可實測
- 同網段自動探索、配對
- 任一電腦可設為主控端，2–3 台共用鍵鼠
- 左／中／右版面，滑鼠跨螢幕邊界切換
- 游標要「絲滑」：主控端游標隱藏、遠端從對應邊界進入
- 英文（預設）／中文
- 剪貼簿共享（文字／圖片）
- 檔案傳輸（拖放＋複製貼上）
- **可操作 Windows 登入畫面 / UAC**（這是架構最大的約束）
- 開源、GPL-3.0-or-later
- 品牌 LAN-KVM，Logo 定案 Seam（兩個螢幕 + 各跨內側一半的左右雙向箭頭）

---

## 2. 技術選型結論

| 項目 | 決策 | 理由 |
|------|------|------|
| 語言/框架 | **Rust + Tauri 2**（UI 尚未做） | Win32 控制完整、常駐資源小、QUIC/TLS 生態成熟、跨平台 |
| 替代方案 | C#/.NET 10 + WinUI 3 | 若確定 Windows-only 且團隊熟 C#，仍可考慮 |
| 不採用 | fork Deskflow | GPL-2.0-only、C++/Qt、舊協定；但可作為行為參考 |
| 協定 | P0 用 28-byte UDP；正式版 QUIC + TLS | P0 先驗證延遲/邊界，正式版才做加密/配對 |
| 探索/配對 | 規劃 mDNS-SD + UDP broadcast + SPAKE2/SAS | 尚未實作 |
| 授權 | GPL-3.0-or-later | 開源；clean-room，沒有直接複製 Deskflow 程式碼 |
| 登入畫面 | Windows Service（LocalSystem）+ SYSTEM session agent + 桌面切換 | 一般使用者程式無法碰 Winlogon secure desktop |
| 驅動 | 不採用 | 簽章/WHQL/BSOD 成本太高；v1 用服務 + hook/SendInput |

詳細評估見 `docs/01-tech-selection.md`、`docs/02-decisions-and-architecture.md`。

---

## 3. 研發時間線（高階）

| 階段 | 內容 | 結果 |
|------|------|------|
| 需求與評估 | 需求拆解、技術棧、登入畫面可行性、風險 | 選定 Rust/Tauri + 服務架構 |
| 品牌 | 5 個極簡 Logo 草案 | 選定 Seam；輸出 SVG/PNG/ICO/ICNS/托盤圖示 |
| P0 骨架 | Rust workspace、core/input/net/platform/agent | `cargo test` 通過 |
| Windows 診斷 | `probe`（session/desktop/DPI/monitor/cursor） | 兩台實機可跑 |
| 輸入 PoC | low-level hook、Raw Input、SendInput、邊界切換 | 兩台實機成功由 A 控制 B |
| 登入畫面 PoC | Windows Service、winlogon token、SYSTEM session agent、桌面 watcher | Default ↔ Winlogon 切換成功 |
| Winlogon hook | 每桌面 worker + hook 計數 | Winlogon 收得到鍵鼠 |
| Winlogon 注入 | 本機封包 → injector thread → SendInput | 登入畫面游標可移動、Caps Lock 可切換 |
| 單機引擎 | 剪貼簿文字/圖片/檔案、監聽、postcard wire format、檔案分塊複製 | 全部 PASS |
| v0.1-local | 測試包、腳本、文件、GitHub private repo | 已提交 |

---

## 4. 目前架構

```
┌──────────────────────────────────────────────┐
│ lankvm-agent.exe（產品骨架）                 │
│  probe / version；不含 hook/keylog/UDP       │
└──────────────────────────────────────────────┘

┌──────────────────────────────────────────────┐
│ lankvm-service.exe（LocalSystem Service）    │
│  - supervisor：監控 active console session   │
│  - 以 winlogon.exe token 啟動 session agent  │
└───────────────────┬──────────────────────────┘
                    │ CreateProcessAsUserW (SYSTEM)
┌───────────────────▼──────────────────────────┐
│ session agent（session 1, SYSTEM）           │
│  - 每 250ms 觀察 OpenInputDesktop            │
│  - Default/Winlogon 切換時重建 worker        │
│  - worker：SetThreadDesktop + hooks           │
│  - injector：另一個 thread，SendInput         │
│  - P0 UDP client（config client_port）        │
└──────────────────────────────────────────────┘
```

### 重要設計決策

1. **hook worker 與 injector 分開**
   - 實測：在掛 low-level hook 的執行緒直接 `SendInput` 會失敗（`inject_fail=1`）。
   - 修正：client 模式**不掛 hook**，另開 injector thread 執行 `SendInput`。
   - injector **不要** `SetThreadDesktop`；實測那樣反而讓 `SendInput` 失敗。`SendInput` 會打到目前 active input desktop；Default/Winlogon 都是。

2. **master 的 liveness guard**
   - 舊版：主控端一到邊界就隱藏游標，若 client 離線會卡住。
   - 現在：必須在 3 秒內收到 PONG 才允許切換；否則印 `client not reachable` 並留在本地。

3. **release 原因修正**
   - 舊版所有 `WM_LANKVM_RELEASE` 都標成 `peer`，包括 timeout。
   - 現在用 `RELEASE_REASON` 區分 `hotkey/peer/timeout/edge/stop`。

4. **角色是手動的**
   - P0 用 `--role master/client`；沒有自動偵測「哪台有實體滑鼠」。
   - 使用 B 的滑鼠要讓 B 當 master；B 當 client 時，B 的滑鼠只會動 B 自己。
   - 本地取回 / 雙向控制 / 動態 master 尚未實作。

---

## 5. 目前可用的功能

### 5.1 單機已驗證

| 功能 | 指令 | 狀態 |
|------|------|------|
| 系統資訊 | `lankvm-agent.exe probe` | PASS |
| 相對注入 | `lankvm-devtools.exe inject-test` | PASS |
| 絕對注入 | `lankvm-devtools.exe abs-test` | PASS（滑鼠靜止時） |
| 剪貼簿文字/圖片/檔案 | `lankvm-devtools.exe clipboard-test [--files ...]` | PASS |
| 剪貼簿監聽 | `lankvm-devtools.exe clipboard-watch` | PASS |
| 本機檔案複製 | `lankvm-devtools.exe file-test --source ... --dest ...` | PASS |
| 本機 P0 注入 | `lankvm-devtools.exe local-winlogon-test` | PASS（滑鼠/Caps Lock） |
| Service 安裝/移除 | `lankvm-service.exe install/uninstall` | PASS |
| Default/Winlogon | `session.log` | PASS |
| Winlogon hook | `session.log` `Winlogon: hooks active` | PASS |
| Winlogon SendInput | 本機封包 | PASS |

### 5.2 跨機已完成但被網路阻擋

A（master）→ B（service client）的 P0 UDP：

- 程式路徑已完成
- B 本機 loopback 可收
- A ping B 正常
- **A→B UDP 封包沒到 B**：最可能是 Apex One/EDR 或 VM 網路政策
- Windows 防火牆不是原因：A 的三個 profile 都是關閉；B 用 `netsh` 可看到 UDP 45871 allow 規則

這個問題必須在乾淨 VM（無 Apex One）或由 IT 放行後才能完成跨機登入畫面測試。

---

## 6. 測試環境與帳密/位置（交接用）

| 角色 | 主機/路徑 | IP | 備註 |
|------|-----------|----|------|
| A | `D:\caron\99_temp\LAN-KVM` | `192.168.137.1` | Windows 防火牆關閉；曾用 debug devtools |
| B | `D:\99_Temp\opencode\LAN-KVM` | `192.168.137.53` | Apex One；Wi-Fi；service 在此 |
| 網路 | 192.168.137.0/24 | — | A 是 gateway `.1`；B ping 得到 A |
| 實體 KVM | 一組 USB 鍵鼠 | — | 切到 A 時 A 是 master；切到 B 時 B 可當 master |

> 不要假設 A/B 名稱；以 IP 與 `Get-NetUDPEndpoint` 判斷哪台在監聽。

---

## 7. 建置與執行

### 7.1 從另一台電腦接手

```powershell
git clone https://github.com/caronyang/LAN-KVM.git
cd LAN-KVM
cargo build -p lankvm-devtools -p lankvm-service -p lankvm-agent
cargo test --workspace
```

需求：

- Rust stable（目前本機 1.86 / MSVC；image 已 pin 0.25.9 以符合 MSRV）
- Windows 10/11
- Node/npm 尚未需要（Tauri UI 未做）

### 7.2 Apex One / 防毒

- release/dev-fast 的 service/devtools 可能被 Apex One 立即刪除
- 目前可用的是 **debug**：
  - `target\debug\lankvm-service.exe`
  - `target\debug\lankvm-devtools.exe`
- 正式版必須簽章 + 白名單；見 `docs/04-av-and-signing.md`
- 不要在正式機器關閉防毒

### 7.3 Service

```powershell
# 管理員 PowerShell
.\target\debug\lankvm-service.exe install
New-Item -ItemType Directory -Force C:\ProgramData\LAN-KVM | Out-Null
Set-Content C:\ProgramData\LAN-KVM\config.txt "client_port=45871"
.\target\debug\lankvm-service.exe stop
.\target\debug\lankvm-service.exe start
```

Logs：

```
C:\ProgramData\LAN-KVM\logs\service.log
C:\ProgramData\LAN-KVM\logs\session.log
```

---

## 8. P0 封包格式（給後續接手）

`crates/lankvm-net/src/lib.rs`

```
28 bytes:
magic "LKVM" [0..4]
version u8 [4]
kind u8 [5]
flags u16 [6..8]
seq u32 [8..12]
a i32 [12..16]
b i32 [16..20]
c i32 [20..24]
d i32 [24..28]
```

種類：

```
1 Hello
2 Switch   a=Y ratio(0..65535), b=entry edge(1=left,2=right)
3 Motion   a=dx, b=dy
4 Button   a=button(1..5), b=down
5 Key      a=scan, b=down, c=extended
6 Wheel    a=delta, b=horizontal
7 Release  a=reason
8 Ping     a=ms
9 Pong     a=ms
```

P0 **沒有加密、沒有配對、沒有 ACK**；正式版要換 QUIC/TLS。

---

## 9. 失敗與修正紀錄（重要）

| 問題 | 原因 | 修正 |
|------|------|------|
| A→B 一直 `client not reachable` | service 沒收到 UDP；B 防火牆規則一開始不存在 | 加規則後仍有問題；`netsh` 證實規則存在，最後判定是 Apex One/EDR/VM 網路 |
| `Get-NetFirewallRule` 查不到規則 | 原則存放區/顯示差異 | 用 `netsh advfirewall firewall show rule name=...` 驗證 |
| 主控端 `OFF (peer)` 但其實是 timeout | 所有 release 訊息共用 `peer` | 加 `RELEASE_REASON` 區分 |
| `remote motion 0/s` | 早期 `ClipCursor` 與 Raw Input 關係未確認 | 實測 Raw Input 在 ClipCursor 下仍運作 |
| hook thread `SendInput` 失敗 | 低階 hook thread 直接注入被拒 | 分離 injector thread |
| injector `SetThreadDesktop` 後 `SendInput` 失敗 | 桌面 attach 反而讓注入失敗 | injector 不 attach；打到 active input desktop |
| service/client 在 Winlogon 無法注入 | 尚未有 SYSTEM/secure desktop 存取 | service + winlogon token + session agent |
| A→B 封包不到 | 不確定 | 仍需 VM/IT 驗證 |
| 只有一組 USB KVM，B 的滑鼠過不去 | B 當時是 client，P0 不捕捉 client 本地輸入 | 手動把 B 設 master；未來做 local takeover |
| release devtools/service 消失 | Apex One 刪除 | 分離 devtools/產品；改用 debug；正式版簽章 |
| 版本資源 link 失敗 | MinGW windres 是 32-bit，和 MSVC x64 衝突 | 改用 Windows SDK `rc.exe` 編譯 `.res` |
| image 0.25.10 無法編譯 | 需要 Rust 1.88，本機 1.86 | pin `image = 0.25.9` |
| clipboard-win trait 匯入錯誤 | 是 `Getter`/`Setter`，不是 `Get`/`Set` | 修正匯入 |
| abs-test 偶發失敗 | 實體滑鼠移動或 service 同時注入 | 加重試；改為手動測試，並在文件註明 |
| PowerShell 呼叫 exe 參數錯亂 | `& $exe @args` 在此環境有時把 exe 路徑當 argv[1] | 改用 `Start-Process -ArgumentList`；dist 腳本已這樣做 |

---

## 10. 目前 repo 結構

```
crates/
  lankvm-core/       # 幾何、佈局、共用型別
  lankvm-input/      # 邊界偵測、座標映射（純邏輯 + 測試）
  lankvm-net/        # P0 封包格式
  lankvm-platform/   # Windows probe/hook/injection/PoC
  lankvm-clipboard/  # 文字/圖片/CF_HDROP、監聽、postcard
  lankvm-files/      # 分塊複製、BLAKE3、atomic rename
  lankvm-agent/      # 產品骨架 probe/version
  lankvm-devtools/   # 開發測試工具（不散布）
  lankvm-service/    # Windows Service + SYSTEM session agent
assets/branding/     # Logo/icons
dist/                # v0.1-local 測試包
docs/                # 技術/測試/AV/研發歷程
```

---

## 11. 明天 VM 測試清單

1. 兩台 VM（Bridged 或同 Internal Switch）互 ping。
2. 解壓 `LAN-KVM-v0.1-local.zip`。
3. B（管理員）：

```powershell
Set-ExecutionPolicy -Scope Process Bypass
.\install-service.ps1 -ClientPort 45871
Get-Content C:\ProgramData\LAN-KVM\logs\session.log -Tail 20
```

4. B 防火牆（若有開）：

```powershell
New-NetFirewallRule -DisplayName "LAN-KVM P0 UDP 45871" `
  -Direction Inbound -Protocol UDP -LocalPort 45871 -Action Allow -Profile Any
```

5. A：

```powershell
.\lankvm-devtools.exe poc --role master --peer <B_IP> --port 45871 --edge right --seconds 180
```

6. 確認 A 出現：

```
[master] client online
```

7. B 按 `Win+L`，等：

```
input desktop -> Winlogon
Winlogon: injector worker installed
Winlogon: injector active
```

8. 推 A 的滑鼠過右邊界，測 B 登入畫面游標；測 Caps Lock。
9. B log 看：

```
Winlogon: input counts ... injected=N inject_fail=0
```

10. 測完 `.\uninstall-service.ps1`。

---

## 12. 尚未完成 / 建議下一步

| 優先 | 工作 | 說明 |
|------|------|------|
| P0 | VM 跨機 UDP | 目前唯一 blocker；乾淨 VM 應可過 |
| P0 | Winlogon 鍵盤完整測試 | Caps Lock 已在本機 Default 驗證；Winlogon 待 VM |
| P1 | 絕對定位接入 P0 | 目前 P0 Motion 是 dx/dy；`move_absolute` 已驗證 |
| P1 | local takeover / 雙向控制 | client 本地滑鼠可一鍵取回；需求「任一電腦可 master」 |
| P1 | QUIC/TLS + mDNS + 配對 | 取代未加密 UDP |
| P2 | 跨機剪貼簿/檔案同步 | 本機引擎已完成；接傳輸層 |
| P2 | 拖放（OLE 虛擬檔） | 高難度 |
| P3 | Tauri UI / 安裝檔 / 自動更新 | 尚未開始 |
| P3 | 多螢幕/混合 DPI 實機 | 目前只有單螢幕 |
| P3 | macOS/Linux | 只有架構預留 |

---

## 13. 接手注意事項

1. **不要提交 token/密碼**。這個 repo 是 private，但仍不應放 secret。GitHub 認證用 `gh auth login` 或 `GH_TOKEN` 環境變數。
2. **不要用 release/dev-fast service 測試**；Apex One 會刪。用 debug 或乾淨 VM。
3. **service 是 LocalSystem**：只裝測試機；測完移除。
4. **P0 UDP 未加密**：不要放在不受信任網段。
5. **角色是手動的**：master 必須是「有實體鍵鼠」那台；不要把 client 的滑鼠當 master。
6. **Winlogon 注入由 injector thread 執行**：不要改回 hook thread 直接 SendInput。
7. **改 hook/injector 後要重新測 Default 與 Winlogon**；`inject_fail` 是最快指標。
8. **abs-test 要離線/靜止測**：實體滑鼠或 service 注入會干擾。
9. **build 後先看 `SHA256SUMS.txt`**；二進位 hash 會隨版本變。
10. **先讀 `docs/07-local-verification.md`**，再讀 `docs/06-service-login-poc.md`。

---

## 14. 常用指令速查

```powershell
# 建置
cargo build -p lankvm-devtools -p lankvm-service -p lankvm-agent

# 測試
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# 單機
.\target\debug\lankvm-devtools.exe probe
.\target\debug\lankvm-devtools.exe abs-test
.\target\debug\lankvm-devtools.exe clipboard-test
.\target\debug\lankvm-devtools.exe clipboard-watch --seconds 15
.\target\debug\lankvm-devtools.exe file-test --source <file> --dest <dir> --overwrite

# Service
.\target\debug\lankvm-service.exe install
.\target\debug\lankvm-service.exe stop
.\target\debug\lankvm-service.exe uninstall
Get-Content C:\ProgramData\LAN-KVM\logs\session.log -Tail 30

# 本機 Winlogon 測試
.\target\debug\lankvm-devtools.exe local-winlogon-test --delay 15 --motion 300
```

---

## 15. 結論

目前已經把「**Windows 服務 + SYSTEM session agent + Winlogon 桌面 + 滑鼠/鍵盤注入**」這條最難的技術路徑打通，並且完成剪貼簿/檔案的本機引擎。唯一還沒打通的是 **A→B 的 UDP 網路路徑**，而 Windows 防火牆已排除，最可能是 Apex One/EDR 或 VM 網路政策。

明天在乾淨 VM 上，先確認 `client online`，再確認 `Winlogon: injector active`，就能完成跨機登入畫面控制。之後才進入 QUIC/TLS、配對、跨機剪貼簿/檔案、UI。
