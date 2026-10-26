# 延遲與「跳回」修正（v1.1）

> 日期：2026-10-26
> 版本：`1.1.0`
> 產物：`dist-app/LAN-KVM-Setup-1.1.0.exe`
> SHA-256：`EABF52D0671A5241A0626CDA8C73C94A2CB3AA3F436D6B0B408D89C74E2CF712`

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
