# LAN-KVM 1.1.1

Reliability release: fewer stalls, no surprise control hand-backs, and a fix for
the 1.1.0 keepalive regression that restarted the session agent every 20–30 s.

## Download

| File | Size | SHA-256 |
|---|---|---|
| `LAN-KVM-Setup-1.1.1.exe` | 2.96 MB | `226B5847E7D822935114C4BBF63AEC3054DC64A9C92E14F076A174DE1EDC5640` |

## What changed

**Networking / feel**

- **10 s liveness window** (was 3 s) and **any packet counts as alive**, including a
  client keepalive. A 1–4 s Wi-Fi stall no longer hands control back.
- **Frame-coalesced mouse motion** on both ends: the master sends at most 250 packets/s
  and the client merges a backlog into one relative move. No more "lag then jump".
- **Edge return is deliberate**: `release_px` default 2 → 10 and two consecutive outward
  packets at the edge are required.
- **IP changes no longer drop a working session**: link-local/loopback/multicast sources
  are ignored, a new address needs two announcements, and the old address is probed
  before reconnecting.
- **Debounced health checks**: the service needs 6 failed probes (30 s) before restarting
  the agent; the app needs 3 misses (15 s) before flipping state.
- **1 MB UDP buffers** on both sides.

**1.1.1 hotfix**

- Keepalives are only sent after real input, and stop after 5 s, so they never target a
  closed probe/engine port.
- `WSAECONNRESET` / `WSAETIMEDOUT` on UDP receive (an ICMP port-unreachable from a closed
  port) is treated as a lost datagram instead of killing the listener.
- If the client listener really dies, the agent exits so the service relaunches it in 5 s
  instead of leaving a deaf process for 30 s.

**Diagnostics**

- `engine.log` every 5 s: `[net] tx/rx/rtt/rtt_max/motion/silent/remote`.
- client `session.log` every 5 s: `[net] rx/max_gap/controlled`.
- Release reason includes how long the peer had been silent.
- `session.log` lines no longer interleave mid-line.
- `system.txt` includes Wi-Fi interface/BSSID/signal and adapter power settings.

## If you still see stalls

Wi-Fi power saving and roaming are the usual culprits. On the client:

```powershell
Set-NetAdapterPowerManagement -Name "Wi-Fi" -AllowComputerToTurnOffDevice Disabled
netsh wlan show interfaces          # prefer 5 GHz and a high rate
powercfg /setactive SCHEME_MIN
```

Driver: Power Save = CAM / Maximum Performance, Roaming Aggressiveness = lowest.
Ethernet on the client is still the single biggest improvement.

---

# 繁體中文

可靠性修正版：減少卡頓、避免控制突然跳回，並修掉 1.1.0 keepalive 造成
session agent 每 20–30 秒重啟的問題。

## 下載

| 檔案 | 大小 | SHA-256 |
|---|---|---|
| `LAN-KVM-Setup-1.1.1.exe` | 2.96 MB | `226B5847E7D822935114C4BBF63AEC3054DC64A9C92E14F076A174DE1EDC5640` |

## 修改內容

**連線與手感**

- **存活判定 3 秒 → 10 秒**，任何封包都算活著；Wi-Fi 停頓 1–4 秒不會再被強制放開。
- **滑鼠封包合併**：master 每 4 ms 最多一包，client 把積壓的 motion 合成一筆。
- **回切要明確**：`release_px` 預設 10，且需連續 2 個向外封包。
- **IP 變更不再拆連線**：忽略 APIPA/loopback/multicast，新位址要連續 2 次公告，
  切換前先探測舊位址。
- **健康檢查去抖**：服務端 6 次失敗（30 秒）、App 端 3 次（15 秒）才翻狀態。
- UDP 緩衝區 1 MB。

**1.1.1 熱修**

- keepalive 只在收到真正的輸入封包後啟動，且 5 秒沒新輸入就停止，不會再打關閉的埠。
- UDP 收到 `WSAECONNRESET / WSAETIMEDOUT`（ICMP port unreachable）視為掉包，不再結束迴圈。
- 若 listener 真的死亡，agent 直接退出，服務 5 秒內重啟（不再留 30 秒殭屍）。

**診斷**

- `engine.log` 每 5 秒 `[net] tx/rx/rtt/rtt_max/motion/silent/remote`。
- client `session.log` 每 5 秒 `[net] rx/max_gap/controlled`。
- 釋放原因附上靜默時間；`session.log` 不再行內交錯。
- `system.txt` 加入 Wi-Fi 介面／BSSID／訊號與網卡省電設定。

## 若還是會卡

```powershell
Set-NetAdapterPowerManagement -Name "Wi-Fi" -AllowComputerToTurnOffDevice Disabled
netsh wlan show interfaces
powercfg /setactive SCHEME_MIN
```

驅動：Power Save = CAM / Maximum Performance、Roaming Aggressiveness = 最低。
client 改有線仍是效果最大的解。
