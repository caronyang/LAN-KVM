# 延遲與「跳回」修正（v1.1）

> 日期：2026-10-26
> 版本：`1.1.4`
> 產物：`dist-app/LAN-KVM-Setup-1.1.4.exe`
> SHA-256：`B9780DDD6704554C2568E4EAF5A0CE016412275EBD67740E8C22B30ED6CB595B`

> **1.1.1 熱修**：1.1.0 新增的 keepalive 會打到已關閉的 UDP 埠，Windows 回應 ICMP
> port-unreachable，client 的接收迴圈以 `WSAECONNRESET / WSAETIMEDOUT` 結束，
> 造成兩台的 session agent 反覆被重啟（每 20–30 秒），使用起來就是「兩邊都卡住」。
> 修正內容見第 6 節。

這一版只做一件事：把實機使用時的**卡頓**與**控制突然跳回**修掉。
所有改動都來自 2026-10-24 的兩份 log（`LAN-KVM-logs-LAPTOP-...-1792815010`）。

---

## 1. 從 log 看到的證據

那段 172 秒的視窗（A 的 `engine.log`）：

| 指標 | 修改前 |
|---|---|
| RTT 中位數 | 8–9 ms |
| RTT p90 | **1613 ms** |
| RTT 最大 | **9577 ms** |
| RTT > 1 秒 | 24 / 145（16.5%） |
| 控制 ON | 8 次 |
| OFF (edge) | 4 次 |
| **OFF (timeout)** | **4 次** |
| `client not reachable` 灌行 | 91 / 339 |

另外：

- B 的來源 IP 曾變成 `169.254.164.33`（APIPA），A 立刻停掉 master 重連，並對不可達位址送封包（`os error 10051`）。
- B 的 service 在重裝期間重啟 3 次，session agent 每 5 秒被換掉（`client health probe failed`）。
- 桌面切到 `Winlogon` 時注入走 loopback TCP。
- B 的 Wi-Fi 是 Realtek 8821CE（1×1 802.11ac），DHCP 重新取得租約、時鐘快 12 分鐘。

結論：**RTT 尖峰 + 3 秒 timeout 是「跳回」主因**；IP 亂跳、健康檢查誤殺是第二、第三名；
「卡」主要來自 motion 沒有合併與 Wi-Fi 本身。

---

## 2. 程式改動

### 2.1 存活判定（master）

`crates/lankvm-platform/src/poc.rs`

| 項目 | 修改前 | 修改後 |
|---|---|---|
| 判定依據 | 只看 PONG | **任何封包**（PONG / keepalive / release） |
| 逾時 | **3 秒** | **10 秒** |
| PING 頻率 | 1 Hz | 控制中 4 Hz、待機 1 Hz |
| PING 路徑 | 與 motion 共用無界佇列 | **直接從 heartbeat 執行緒送出**，RTT 用實際送出時間 |
| 「不可達」log | 每次邊界觸發都印 | 最多每 2 秒一行 |

client 端（`lankvm-service/src/net.rs`、`poc.rs` 的 PoC client）新增**每 500 ms 的主動 keepalive**
（`Pong` 且 `a=0`），所以就算 master 的 PING 全部掉包，只要回程還通就不會被判死。

### 2.2 滑鼠封包合併（兩端）

| 位置 | 修改前 | 修改後 |
|---|---|---|
| master 送出 | 每個 raw input 事件一個 UDP 封包（可達 1000/s） | 累加 dx/dy，**每 4 ms 最多一包**（250 Hz），且保留總位移 |
| client 注入 | `while recv() { SendInput }` 逐筆補送 | 一次把佇列中的 Motion **合併成一筆**再注入 |

停頓 4 秒後原本會補送 400 筆舊座標，游標「慢半拍後爆衝」；現在只補一個總位移，位置立刻正確。

### 2.3 邊界回切不再手滑

- `release_px` 預設 **2 → 10**（服務端另外 `max(6)` 保護舊設定）。
- 需要**連續 2 個向外封包**且游標真的在邊緣才回切。
- 桌面切換（鎖定/UAC）時重設累計值。

### 2.4 IP 變更處理

`crates/lankvm-app/src/net.rs`

- 忽略 loopback / 0.0.0.0 / multicast / broadcast / **169.254/16（APIPA）** / IPv6 link-local。
- 新位址要**連續 2 次公告**才採用（`ip_change_confirmed`）。
- 採用新位址時**先探測舊位址**（UDP ping 資料埠）；舊位址還活著就**保持 session**，
  只更新設定，等下次重連才換。
- 單元測試：`cargo test -p lankvm-app --bins`（3 個測試涵蓋過濾與二次確認）。

### 2.5 健康檢查不再誤殺

| 位置 | 修改前 | 修改後 |
|---|---|---|
| SYSTEM 服務 → agent | 5 秒一次、300 ms、**3 次失敗就殺 agent** | 5 秒一次、500 ms、**每次重試 2 次**、**6 次失敗（30 秒）**才重啟 |
| App → 本機服務 | 250 ms、單次 | 500 ms、每次重試 2 次 |
| App 狀態翻轉 | 立刻就變 `not ready` | **連續 3 次失敗（15 秒）**才翻；恢復立即翻 |

client 角色恢復時才會送 `session_reset`，所以 master 不會再因為一次探測抖動被停掉。

### 2.6 其他

- master / client 的 UDP socket 緩衝區加大到 1 MB。
- `session.log`：每個 process 的每行改成**一次寫入**、加 pid、加鎖，
  不再出現 `[[1792815698] ...release received from master]` 這種交錯。
- `system.txt` 新增：`netsh wlan show interfaces`、`netsh wlan show networks mode=bssid`、
  `Get-NetAdapterPowerManagement`、Clipboard 防火牆規則。
- `engine.log` 每 5 秒一行：

```
[net] tx=42 rx=58 rtt=7ms rtt_max=23ms motion=96/s silent=38ms remote=true
```

- `session.log`（client）每 5 秒：

```
[net] rx=213 max_gap=41ms controlled=true
```

- `remote control OFF (timeout, peer silent 10234 ms)`：釋放原因加上當時靜默多久。
- 安裝程式會把舊 `config.txt` 的 `release_px=2` 遷移成 10。

---

## 3. 環境建議（不是程式能解的部分）

從 log 看，B 的 Wi-Fi 是主要變數。建議依序做：

```powershell
# 1) 關閉網卡省電（系統層）
Set-NetAdapterPowerManagement -Name "Wi-Fi" -AllowComputerToTurnOffDevice Disabled

# 2) 確認目前連線與速率（5 GHz、rate 越高越好）
netsh wlan show interfaces

# 3) 電源計畫改高效能
powercfg /setactive SCHEME_MIN
```

驅動層（Realtek 8821CE）：

- **Power Save Mode = CAM / Maximum Performance**
- **Roaming Aggressiveness = 最低**
- 若有的話關閉 **Scan when connected**

其他：

- B 若能接**網路線**，優先有線；這是延遲最有效的解。
- 兩台開 NTP（這次 B 快 12 分鐘，log 對不起來）。
- Apex One / EDR 對 `lankvm*.exe` 與 UDP 45870–45872 的即時掃描可能加延遲。

---

## 4. 怎麼驗證

1. 兩台安裝 `LAN-KVM-Setup-1.1.0.exe`（設定會保留；`release_px` 會自動遷移）。
2. 不要重裝/重啟，正常使用 5 分鐘，刻意快速甩動滑鼠、在邊緣附近活動。
3. 觀察 `engine.log`：

```
[net] tx=... rx=... rtt=...ms rtt_max=...ms motion=.../s silent=...ms remote=true
```

- `silent` 應維持在 1 秒內。
- `rtt_max` 若經常 > 1000 ms，代表網路仍在停頓（先做第 3 節）。
- `OFF (timeout, ...)` 應該從每 40 秒一次降到幾乎不出現。

4. 在 client 的 `session.log` 看 `[net] rx=... max_gap=...ms`：`max_gap` 就是封包最大間隔。

---

## 5. 相容性

- 新舊版可以互通：舊 client 不會送 keepalive，但新 master 仍有 4 Hz PING；
  舊 master 收到新 client 的 `a=0` keepalive 只會多印一行很大的 RTT，不影響功能。
- `release_px` 在 client 端生效（`C:\ProgramData\LAN-KVM\config.txt`）；
  App 的 UI 欄位一併改預設 10，舊 `app.json` 的 2 會自動遷移。

---

## 6. 1.1.1 熱修：keepalive 造成 agent 反覆重啟

### 6.1 症狀

- 一開始正常，過一小段時間**兩邊的滑鼠都不動、游標像消失**，一陣子又突然可用。
- `app.log`（兩台）每 15–20 秒 `client engine not ready` → `ready` → `not ready`。
- B 的 `service.log`：

```
client health probe failed (1/6) ... (6/6)
restarting session agent: UDP client not responding
```

- B 的 `session.log`：

```
UDP receive failed: 遠端主機已強制關閉一個現存的連線 (os error 10054)
UDP client stopped
```

### 6.2 根因

1.1.0 讓 client 每 500 ms 主動送 keepalive（`Pong`）給「最後一次收到封包的來源位址」。
但那個位址可能是：

- 已經關閉的本機健康檢查 socket（服務每 5 秒用一個臨時 UDP socket ping 自己），或
- 已經結束/重啟的 master engine（ephemeral port 已釋放）。

UDP 送到關閉的埠，Windows 會回 **ICMP port unreachable**；下一個 `recv_from`
就會得到 `WSAECONNRESET (10054)` 或 `WSAETIMEDOUT (10060)`。
舊程式把這個當成致命錯誤 → 接收迴圈結束 → **process 還活著但沒有 listener** →
服務的健康檢查失敗 → 30 秒後殺掉重啟 → 新 agent 又被 keepalive 打死 → 無限循環。

master 端同理：PING 打到已關閉的資料埠時，接收執行緒也可能被 ICMP 打死。

### 6.3 修正

| 位置 | 修正 |
|---|---|
| client keepalive | 只有收到**真正的輸入封包**（Hello/Switch/Motion/Button/Key/Wheel/Release）才啟動；**5 秒內沒有新輸入就停止**。單純的 Ping（健康檢查）不再啟動 keepalive |
| UDP 錯誤處理 | `WouldBlock / TimedOut / ConnectionReset / ConnectionAborted / ConnectionRefused` 一律視為「掉一個封包」繼續，不再結束迴圈 |
| agent 生命週期 | 若接收迴圈真的因其他錯誤結束，**讓 agent 直接退出**，服務 5 秒內就會重啟（不再留下 30 秒的殭屍） |
| master 接收 | 同樣忽略 ICMP 造成的錯誤 |
| App 的 discovery / control UDP | 同樣忽略上述錯誤，避免控制通道被 ICMP 打掉 |

### 6.4 驗證

- 用一個臨時 UDP socket 送一個 Motion 封包後立刻關閉（模擬「master 消失」），
  再讓 client 對它送 keepalive：**client 程序仍存活**（修正前會以 10060 結束）。
- master 在目標埠完全沒有 listener 的情況下啟動 3 秒（每次 PING 都收到 ICMP），
  之後再啟動 client：master 正確顯示 `client online`、`rtt=1 ms`（接收執行緒沒死）。

### 6.5 這次一併調整

- `release_px` 舊值 1 或 2 一律視為 10（`config.txt` 與 `app.json` 都適用）。

---

## 7. 1.1.2：被控端 Shift + `/` 只出現 `/`

### 7.1 症狀

在 B（被控制的那台）按 **Shift + `/`** 不會出現 `?`，只會出現 `/`。

### 7.2 分析

用封包直接對 client engine 注入，逐一驗證：

| 送出的內容 | 結果 |
|---|---|
| 正常 Shift 按下 + `/` | `?` ✓ |
| `/` 封包帶 **extended**（numpad `/`） | **`/`**（Shift 完全無效） |
| 只有 `/`（Shift 封包遺失） | **`/`** |

所以只有兩種可能：

1. **`/` 被封成 numpad `/`**（extended 0x35）→ 它本來就不受 Shift 影響。
2. **Shift 的 keydown 封包遺失/被忽略**（UDP 掉包、或在 Switch/Release 時被放掉）。

### 7.3 修正

- **master 在每個 Key 封包帶上目前的 Shift 狀態**（`packet.flags` bit 0）。
- **client 在注入任何按鍵前同步 Shift**：
  - 若 master 說 Shift 是 down 而本地不是 → 先補一個 LShift down。
  - 反之 → 先補 LShift up。
- **Shift 一律用同一個 scan code（0x2A）驅動**：實體左右 Shift 都轉成同一個狀態，
  不會再出現「按了 RShift、放開時只放 LShift」的半按狀態。
- **Shift 的 extended 旗標一律忽略**（Shift 永遠不是 extended key）。
- Winlogon helper（登入畫面）與 PoC client 套用同樣邏輯。

副作用：client 端只看得到 LShift，玩遊戲時無法區分左右 Shift；打字不受影響。

### 7.4 診斷 log（`session.log`，最多每 5 秒一行）

```
Default: shift state resynced (down)      ← 有 Shift 封包遺失，已自動補
Default: numpad divide (extended 0x35) injected   ← 收到的是數字鍵盤的 '/'
```

若你再次遇到，請把這兩行（以及發生時間）一起給我。

---

## 8. 1.1.3：被控端斷線時游標不回、與緊急取回

### 8.1 症狀

控制 B 時，B 斷線或 B 的服務停止，**A 的滑鼠不會回來**（游標被釘住/隱藏），
只能重開 App 才恢復。

### 8.2 根因

原本的 10 秒 timeout 是跑在 heartbeat 執行緒裡，而該執行緒會呼叫 `socket.send()`。
UDP socket 是**阻塞模式**；當 B 消失、ARP/ICMP 卡住或系統緩衝耗盡時，
`send()` 會卡住（A 的 `app.log` 就看過 `os error 10055` = WSAENOBUFS），
heartbeat 執行緒因此停止 → **timeout 永遠不會觸發**。

（另外 `engine.log` 每次啟動都被覆蓋，所以上一輪卡住的紀錄消失了。）

### 8.3 修正

| 層級 | 修正 |
|---|---|
| socket | master / client 的 UDP 都設 **100 ms send timeout**，送不出去就放棄，不會卡住 |
| engine watchdog | **獨立執行緒**只讀 atomics、不做 I/O；10 秒沒收到封包就送出 release。若 5 秒後 `REMOTE` 仍是 true（表示主迴圈卡死），**直接結束 engine 行程**，由作業系統強制釋放 ClipCursor / 隱藏游標 / overlay |
| 緊急熱鍵 | App（托盤行程）註冊 **`Ctrl+Alt+Shift+R`**：即使 engine 卡死也能作用。它會砍掉 engine（必要時 kill）並在 60 秒內不自動重連 |
| 托盤 / UI | 托盤選單「強制取回滑鼠」；UI 右上角紅色「強制取回滑鼠」按鈕；`POST /api/emergency` |
| 結束程式 | `/api/quit` 與托盤結束前都會先做一次 emergency release |
| client | 15 秒沒收到 master 任何封包時，**放掉已按下的按鍵**（避免 Ctrl/Shift 卡住），並停止注入 |
| 診斷 | `engine.log` 改為**附加**（每次啟動寫一行 `===== engine ... start =====`），超過 8 MB 才清空 |

熱鍵：

| 熱鍵 | 作用 | 何時有效 |
|---|---|---|
| `Ctrl+Alt+Shift+Q` | 正常釋放（engine 的 LL hook） | engine 正常時 |
| `Ctrl+Alt+Shift+R` | **強制**取回：終止 engine、清掉游標狀態、60 秒不重連 | engine 卡死時也能用 |

### 8.4 驗證

- `app.log` 應出現 `tray: emergency hotkey Ctrl+Alt+Shift+R registered`。
- 按熱鍵或呼叫 API 後：`EMERGENCY release (hotkey/api)`、`engine master stopped`。
- 這次也實測：API 觸發後 engine 行程結束、session 清除。

---

## 9. 1.1.4：1.1.3 的 Shift 反向問題

### 9.1 症狀

1.1.3 之後，**左右 Shift + `/` 都打不出 `?`**（只出現 `/`），比 1.1.2 更嚴重。

### 9.2 兩個原因

1. **版本不一致**
   1.1.2 的 master 不送 Shift 旗標（`flags = 0`），1.1.3 的 client 把「沒有旗標」
   解讀成「Shift 是放開的」，於是在 `/` 之前**主動把 Shift 放掉**。

2. **LL hook 會吞掉 async key state**
   為了把按鍵只送給對方，hook 回傳 1（吞掉）。這會讓 `GetAsyncKeyState(VK_SHIFT)`
   **永遠回報未按下**，所以每一顆按鍵都帶著「Shift = up」，client 又把 Shift 放掉。

### 9.3 修正

- master 自己追蹤 Shift 狀態（`SHIFT_DOWN`），不再依賴 `GetAsyncKeyState`；
  進入控制時用當下的 async state 初始化（切換前就按著 Shift 也正確）。
- 旗標新增 **valid bit**：`bit0 = Shift down`、`bit1 = 狀態有效`。
  舊 master（`flags = 0`）→ client **不做同步**，維持 1.1.2 的行為。
- engine 的 LL hook 除了 `Ctrl+Alt+Shift+Q`，**也接受 `Ctrl+Alt+Shift+R`**
  （即使 App 是舊版或沒在跑，engine 還能自救）。
- client 端 15 秒沒收到 master 封包時，除了放掉按鍵，也會送一個 Release 封包，
  讓還活著的 master 一併釋放（不再默默停止注入）。

### 9.4 驗證矩陣（本機以封包直接測試）

| 情境 | 結果 |
|---|---|
| 舊 master（flags=0）+ 左右 Shift + `/` | `?` ✓ |
| 新 master（flags=3）+ Shift + `/` | `?` ✓ |
| 新 master，Shift 封包遺失（flags=3，沒有 Shift keydown） | `?` ✓（自動補 Shift） |
| 新 master，沒按 Shift（flags=2）+ `/` | `/` ✓ |

> 注意：**兩台都要安裝 1.1.4**。混用 1.1.2/1.1.4 可以運作（valid bit 保護），
> 但緊急熱鍵 `Ctrl+Alt+Shift+R` 只有 1.1.3 以後的 App 才有。
