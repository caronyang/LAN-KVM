# 新場域測試計畫（v0.1-local 之後）

> 日期：2026-10-23  
> 基準版本：`v0.1-local`（commit `28dca8a`）  
> 目的：在沒有 Apex One／沒有舊 VM 網路政策的場域，把上一個場域唯一卡住的 **A→B UDP** 打通，並完成跨機與登入畫面驗收。  
> 前置閱讀：`docs/07-local-verification.md`、`docs/08-development-history.md`

---

## 0. 目前基準（先確認不要重做）

| 項目 | 狀態 | 說明 |
|------|------|------|
| 單機 probe／注入／剪貼簿／檔案 | PASS | 已於上一個場域驗證 |
| Service + SYSTEM agent + Default/Winlogon 切換 | PASS | 單機可切換桌面 |
| Winlogon hook／SendInput | PASS | 本機封包可移動登入畫面游標、切 Caps Lock |
| 跨機 UDP 握手／邊界切換 | **部分** | 2026-10-21 曾在 A/B 實機 PASS（RTT 3–9 ms），之後同環境被 EDR/網路政策擋住 |
| 跨機 Winlogon | **未驗證** | 只差 A→B 封包到達 |
| QUIC/TLS／配對／跨機剪貼簿／UI | 未實作 | 現階段不用測 |

本次（2026-10-23）在目前這台機器上的實測：

- 主機：`LAPTOP-0MC99DAQ`，Windows 11 26200
- 網路：Wi-Fi `192.168.68.68`（DHCP）；另有 Tailscale `100.73.78.76`、VirtualBox host-only `192.168.56.1`
- 防火牆：Domain/Private/Public **全部開啟**（上一個場域 A 是關閉的，這裡要自己加規則）
- 防毒：McAfee AP Service 執行中（不是 Apex One）
- Rust 工具鏈：**這台沒有** `cargo`／`rustc`
- `dist/` 三支 exe 的 SHA-256 與 `SHA256SUMS.txt` 一致
- 已用 `dist` 的預建 binary 跑本機 loopback：`[master] client online`、RTT 0–2 ms → **PASS**

> 結論：**不需要先建置**，直接複製 `dist/` 到兩台機器就能測。只有要改程式時才需要安裝 Rust。

---

## 1. 測試前準備（約 10 分鐘）

1. 兩台 Windows 機器（以下稱 A = master／有實體鍵鼠、B = client／被控端），接**同一個 AP／Switch**。
2. 兩台都複製 `dist/`（或解壓 `LAN-KVM-v0.1-local.zip`）。
3. 兩台各自開一般 PowerShell 與**管理員 PowerShell** 各一個。
4. 先記錄：

```powershell
hostname
Get-NetIPAddress -AddressFamily IPv4 |
  Where-Object { $_.IPAddress -notlike '127.*' -and $_.IPAddress -notlike '169.254.*' } |
  Select-Object IPAddress, InterfaceAlias
```

5. 確認 binary 沒被防毒改動（在 `dist` 內）：

```powershell
Get-FileHash .\*.exe -Algorithm SHA256 | Select-Object Hash, Path
```

預期（與 `SHA256SUMS.txt` 相同）：

```
379AC4E60E2A4B0CC6F9835CBC092C10110555375018296546888ED8BC1F7302  lankvm-agent.exe
D50CB2D920E0F0FB665B4D3BDDAE1CAB1787A547B1CF4B0F8AA6E794D507E3F5  lankvm-devtools.exe
E2815155B40B7AC7B83E08DB214ACAD20C0B0D38BC7D866AF2A6CEC76C552A18  lankvm-service.exe
```

> A 建議直接接一組 USB 鍵鼠，B 不接鍵鼠；若只有一組實體 KVM，先做功能驗證，手感／延遲等最後再用乾淨配置驗收。

---

## 2. Gate 0：網路可達性（**這次最重要**）

> 先用 `dist/net-check.ps1` 把「網路」和「LAN-KVM」切開。這個腳本只送 28-byte `LKVM` Hello，聽端原封不動回傳，不需要 Rust、不需要裝 service。

> **PowerShell 執行原則**：若出現「因為這個系統上已停用指令碼執行，所以無法載入…….ps1」，改用：
>
> ```powershell
> Set-ExecutionPolicy -Scope Process Bypass -Force
> ```
>
> 或直接跑 `.\run-net-check.cmd …`（它用 `-ExecutionPolicy Bypass` 啟動，不會改機器設定）。從 ZIP 解壓的檔案若被標記，先 `Get-ChildItem .\*.ps1 | Unblock-File`。

### 2.1 兩台都看現況

```powershell
.\net-check.ps1 -Mode info -Port 45871
```

要看：IPv4、防火牆 profile、`LAN-KVM` 規則、誰佔用 UDP 45871、EDR 服務。

### 2.2 B（被控端，管理員）開防火牆並監聽

```powershell
# 方法 1：直接跑（先在這個視窗允許一次）
Set-ExecutionPolicy -Scope Process Bypass -Force
.\net-check.ps1 -Mode listen -Port 45871 -Seconds 120 -AddFirewallRule

# 方法 2：不碰執行原則
.\run-net-check.cmd -Mode listen -Port 45871 -Seconds 120 -AddFirewallRule
```

> `-AddFirewallRule` 會建立 `LAN-KVM P0 UDP 45871`（Inbound/UDP/Any profile）。如果公司政策不允許自行開規則，請 IT 開，不要關防火牆。

### 2.3 A（主控端）送封包

```powershell
Test-NetConnection <B_IP> -InformationLevel Quiet     # ICMP
.\net-check.ps1 -Mode send -Peer <B_IP> -Port 45871 -Count 10
```

**PASS 條件**

- A：`RESULT: PASS - 10/10 replies`，RTT 平均值 < 20 ms（Wi-Fi 可接受）
- B：每秒看到 `28 bytes from <A_IP>`，且封包內容以 `LKVM` 開頭

**FAIL 對照**

| 現象 | 判斷 | 處理 |
|------|------|------|
| ping 就不通 | 不同網段／AP client isolation／防火牆擋 ICMP | 換同一個 AP、關 AP isolation、確認 subnet mask |
| ping 通、UDP 全 timeout | B 防火牆未開、EDR 擋 UDP、AP 擋 UDP | 看 2.1 規則與 EDR；用 `netsh advfirewall firewall show rule name="LAN-KVM P0 UDP 45871"` 確認 |
| 只掉幾個封包 | Wi-Fi 省電／漫遊 | 接有線、關網卡省電後重測 |
| B 收到但 A 沒 reply | 回程被擋 | 兩台都加規則（本腳本預設只加 B） |
| B 的 UDP 45871 被佔用 | 舊 service 還跑著 | `Get-NetUDPEndpoint -LocalPort 45871`，先停 service |

### 2.4 備援路徑：Tailscale

若同網段 UDP 被 AP/EDR 擋死，兩台都登入同一個 Tailscale tailnet，改用 `100.x` 位址重跑 2.2/2.3。

- 優點：可立刻驗證協定與登入畫面，不必等 IT。
- 缺點：延遲不代表區網表現，不能拿來做手感驗收。
- 注意：防火牆仍要允許 UDP 45871；`lankvm-devtools poc` 綁 `0.0.0.0`，用 Tailscale IP 當 `--peer` 即可，但**不要同時把 LAN IP 和 Tailscale IP 混用**。

**Gate 0 通過才往下走。** 若 Gate 0 失敗，後面的 LAN-KVM 測試沒有意義。

---

## 3. Gate 1：單機自我測試（確認新場域沒有把功能擋掉）

在 B（或任一台）一般 PowerShell：

```powershell
Set-ExecutionPolicy -Scope Process Bypass
.\run-local-tests.ps1
```

再手動跑（滑鼠靜止、service 停止）：

```powershell
.\lankvm-devtools.exe abs-test
```

**PASS**：`absolute cursor test PASS`（delta 0 px）。

本機 P0 loopback（不用第二台）：

```powershell
# 視窗 1
.\lankvm-devtools.exe poc --role client --port 45999 --seconds 15

# 視窗 2
.\lankvm-devtools.exe poc --role master --peer 127.0.0.1 --port 45999 --edge right --seconds 8
```

**PASS**：master 顯示 `client online`、`[latency] rtt=` 有數字；client 顯示 `master connected from 127.0.0.1`。

> 這一項在目前機器已 PASS，換場域後仍建議重跑一次，當作「binary 沒被防毒破壞」的基準。

---

## 4. Gate 2：跨機 P0（手動角色）

### 4.1 啟動

B（管理員）：

```powershell
.\install-service.ps1 -ClientPort 45871
Get-Content C:\ProgramData\LAN-KVM\logs\session.log -Tail 20
```

預期：

```
P0 UDP client listening on 0.0.0.0:45871
Default: injector active
```

> 若 service 被防毒刪除，先看 `docs/04-av-and-signing.md`；不要關防毒。

A：

```powershell
.\lankvm-devtools.exe poc --role master --peer <B_IP> --port 45871 --edge right --seconds 300
```

預期順序：

```
[master] peer <B_IP>:45871, edge Right
[master] client online
[latency] rtt=... ms
```

### 4.2 測試項目與判定

| # | 動作 | 預期 | 判定 |
|---|------|------|------|
| 2-1 | A 滑鼠推過右邊界 | A：`remote control ON (Right edge, entry ratio ...)`；B：`controlled; entry at (0, y) from left` | ☐ |
| 2-2 | B 游標是否從左邊界對應高度進入 | Y 比例正確、不明显跳動 | ☐ |
| 2-3 | A 游標隱藏／釘住 | 不會跑出 A 的螢幕、不會顯示在 B | ☐ |
| 2-4 | **把 A 的滑鼠往左推**，讓 B 上的游標回到 B 的左邊界（越界 2px 即回切；B 本地鍵鼠不影響狀態） | A：`OFF (edge)`、游標恢復；B：`cursor reached entry edge -> released to master` | ☐ |
| 2-4b | 反向版面：A 在 B 右側時改 `--edge left` | 進入點變 B 的右邊界，回切同樣立即 | ☐ |
| 2-5 | 按 `Ctrl+Alt+Shift+Q` | A 立即取回，B 顯示 released | ☐ |
| 2-6 | 拔掉 B 網路／停止 service，再從 A 推邊界 | A 顯示 `client not reachable`，**不得**進入 remote、游標不得卡住 | ☐ |
| 2-7 | A 直接關閉 poc（Ctrl+C） | B 不會卡在注入狀態 | ☐ |
| 2-8 | 連續進出 20 次 | 無卡鍵、無游標殘留 | ☐ |
| 2-9 | 兩台螢幕解析度不同 | 邊界與比例仍正確（目前相對注入，手感可能不同） | ☐ |

> **回切（2-4）操作重點**：B 的本地鍵鼠**不會**改變狀態。回切是把控制 B 的游標（A 的滑鼠）推回 B 的進入邊界；越界 2px 就送 Release，不再需要多推 30px。若座位左右相反，A 改用 `--edge left`（版面設定，見 `docs/02` 4.5）。

### 4.3 記錄

每次測試記下：RTT 平均／最大、`remote motion N/s`、有無 `inject_fail`、解析度／DPI、有線或 Wi-Fi。

---

## 5. Gate 3：登入畫面與 UAC（跨機）

前提：Gate 2 的 `client online` 已穩定。

1. 保持 A 的 master 執行中。
2. B 按 `Win+L`，等 `session.log` 出現：

```
input desktop -> Winlogon
Winlogon: injector worker installed
Winlogon: injector active
```

3. A 推過右邊界，測：

| # | 測試 | 預期 |
|---|------|------|
| 3-1 | 登入畫面移動游標 | 游標會動，不延遲爆增 |
| 3-2 | Caps Lock | LED／狀態可切換（測完記得切回） |
| 3-3 | 輸入錯誤密碼後 Enter | 有鍵入反應、出現認證失敗（**不要用真實密碼做自動化測試**） |
| 3-4 | 解鎖回 Default | log 出現 `input desktop -> Default`、`Default: injector active` |
| 3-5 | 開一個「以管理員身分執行」的程式觸發 UAC | 可在 UAC 畫面操作（同一 secure desktop 機制） |
| 3-6 | 快速使用者切換到另一個帳號 | desktop/session 切換不崩、可恢復 |

4. 看 B：

```powershell
Get-Content C:\ProgramData\LAN-KVM\logs\session.log -Tail 40
```

**PASS**：`injected` 持續增加、`inject_fail=0`、沒有 hook 計數停滯。

**FAIL**：`inject_fail>0` → Winlogon `SendInput` 被安全政策擋，往 UIAccess／簽章方向處理（見 `docs/04-av-and-signing.md`）。

5. 測完（管理員）：

```powershell
.\uninstall-service.ps1
```

---

## 6. Gate 4：輸入完整度矩陣

在 B 開記事本／瀏覽器文字框，A 逐項操作：

### 6.1 鍵盤

| # | 項目 | 判定 |
|---|------|------|
| 4-1 | 英文字母、數字、符號（含 Shift 組合） | ☐ |
| 4-2 | Enter／Backspace／Delete／Tab／Esc | ☐ |
| 4-3 | 方向鍵、Home/End/PageUp/PageDown、Insert | ☐ |
| 4-4 | Ctrl／Alt／Shift／Win 單獨與組合（複製貼上、Alt+Tab、Win+D） | ☐ |
| 4-5 | F1–F12、PrtScr、Num Lock、Scroll Lock | ☐ |
| 4-6 | 注音／倉頡／拼音輸入中文 | ☐ |
| 4-7 | Emoji 選字窗 | ☐ |
| 4-8 | 連續快速打字 30 秒 | ☐ |
| 4-9 | 測完放開所有鍵，確認 B 無卡鍵 | ☐ |

> 注音／IME 是 P0 已知風險：目前送的是 scan code，IME 選字視窗屬應用層，通常可用，但需要實測。

### 6.2 滑鼠

| # | 項目 | 判定 |
|---|------|------|
| 4-10 | 左／右／中鍵、雙擊、拖曳選取 | ☐ |
| 4-11 | 滾輪垂直、`Shift`+滾輪水平 | ☐ |
| 4-12 | 側鍵（XButton1/2） | ☐ |
| 4-13 | 拖放檔案到資料夾（本機檔案總管） | ☐ |
| 4-14 | 1000 Hz 滑鼠／藍牙滑鼠 | ☐ |

---

## 7. Gate 5：穩定度與效能

| # | 測試 | 方法 | 判定 |
|---|------|------|------|
| 5-1 | 30 分鐘連續使用 | 正常操作＋反覆進出邊界 | 無卡鍵、無記憶體持續上升 |
| 5-2 | 2 小時 soak | 閒置＋每 5 分鐘進出一次 | 程序還活著、log 無 error |
| 5-3 | 24 小時 soak | 睡前啟動 | 隔天可正常操作 |
| 5-4 | 延遲 | `[latency] rtt=` 取 p50/p95 | 有線 p95 < 15 ms（正式版目標） |
| 5-5 | CPU／記憶體 | 工作管理員觀察 service 與 devtools | 閒置 CPU 接近 0 |
| 5-6 | 網路中斷 10 秒 | 停用再啟用網卡 | 自動恢復或明確失敗，不卡游標 |
| 5-7 | service 重啟 | `lankvm-service.exe stop/start` | master 重新 `client online` |
| 5-8 | Wi-Fi 漫遊 | 走過 AP 覆蓋邊界 | 不掉鍵、不斷線 |
| 5-9 | 多螢幕／混合 DPI | A 接雙螢幕，B 125%/150% | 邊界與 Y 映射正確 |

---

## 8. 現階段**不要**測（功能還沒接上）

以下在程式裡只有本機引擎或尚未實作，測了只會得到「沒有反應」：

- 跨機剪貼簿同步（文字／圖片／檔案）
- 跨機檔案拖放
- 絕對定位游標（`abs-test` 已驗證 API，但 P0 協定仍送 dx/dy）
- mDNS 自動探索、配對、QUIC/TLS
- client 本地滑鼠「取回控制」／雙向 master
- Tauri UI、安裝檔、自動更新
- macOS／Linux

---

## 9. 結果記錄模板

每輪測試複製以下內容到 `docs/10-test-log-YYYYMMDD.md`（或直接貼在 issue）：

```
日期：
場域／網路：        （SSID／switch、subnet）
A：hostname / IP / OS / 有線或 Wi-Fi
B：hostname / IP / OS / 有線或 Wi-Fi
防火牆：A on/off、B on/off、規則是否建立
EDR：
binary hash 是否符合：

Gate 0 net-check：PASS/FAIL，loss，min/avg/max RTT
Gate 1 單機：PASS/FAIL
Gate 2 跨機：PASS/FAIL，失敗項目編號
Gate 3 Winlogon：PASS/FAIL，inject_fail
Gate 4 輸入：PASS/FAIL，失敗項目編號
Gate 5 穩定度：PASS/FAIL

log：
  C:\ProgramData\LAN-KVM\logs\service.log
  C:\ProgramData\LAN-KVM\logs\session.log
```

收 log：

```powershell
$out = Join-Path $env:USERPROFILE "Desktop\lankvm-logs-$(Get-Date -f yyyyMMdd-HHmm)"
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item C:\ProgramData\LAN-KVM\logs\*.log $out -ErrorAction SilentlyContinue
ipconfig /all > "$out\ipconfig.txt"
Get-NetFirewallProfile | Out-File "$out\firewall.txt"
Get-NetUDPEndpoint -LocalPort 45871 | Out-File "$out\udp45871.txt"
```

---

## 10. 通過後的開發順序（依測試結果調整）

1. **Gate 0/2 過、Gate 3 過** → 開始把 P0 UDP 換成 QUIC/TLS，做 mDNS + SPAKE2/SAS 配對。
2. **Gate 3 `inject_fail>0`** → 先處理 UIAccess／簽章，不要急著做 UI。
3. **手感不佳** → 把已驗證的 `move_absolute` 接進 P0（`Switch` 封包帶 entry 座標，Motion 改絕對座標）。
4. **需要「任一台上手」** → 做 local takeover（client 偵測本地輸入→自動交還），这是產品需求，不是 P0。
5. **跨機剪貼簿／檔案** → 本機引擎已完成，接上新的 QUIC 串流即可。
6. **最後**才做 Tauri UI／安裝檔／自動更新。

---

## 11. 一句話總結

**先跑 Gate 0 的 `net-check.ps1`，看到 `10/10 replies` 再裝 service。**  
只要 Gate 0 過，剩下的就是照 Gate 2 → Gate 3 → Gate 4 → Gate 5 的順序把上一版已驗證過的流程在新場域重跑一次；這次的目標不是新功能，而是把「跨機 + 登入畫面」這條路正式驗收。
