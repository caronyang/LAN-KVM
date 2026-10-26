# LAN-KVM 1.1.0

Reliability release: fewer stalls and no more surprise control hand-backs.

## Download

| File | Size | SHA-256 |
|---|---|---|
| `LAN-KVM-Setup-1.1.0.exe` | 2.96 MB | `EABF52D0671A5241A0626CDA8C73C94A2CB3AA3F436D6B0B408D89C74E2CF712` |

## What changed

- **10 s liveness window** (was 3 s) and **any packet counts as alive**, including a new
  500 ms keepalive from the client. A 1-4 s Wi-Fi stall no longer hands control back.
- **Frame-coalesced mouse motion** on both ends: the master sends at most 250 packets/s
  (4 ms window) and the client merges a backlog into one relative move. No more
  "lag then jump" after a stall.
- **Edge return is deliberate**: `release_px` default 2 → 10, and two consecutive outward
  packets at the edge are required. Brushing the edge no longer releases.
- **IP changes no longer drop a working session**: link-local (169.254) / loopback /
  multicast sources are ignored, a new address needs two announcements, and the old
  address is probed before reconnecting.
- **Debounced health checks**: the service needs 6 failed probes (30 s, 2 tries each)
  before restarting the agent; the app needs 3 misses (15 s) before flipping state.
- **Bigger UDP buffers** (1 MB) on both sides.
- **Logs**: `[net] tx/rx/rtt/rtt_max/silent` every 5 s in `engine.log`,
  `[net] rx/max_gap` in the client's `session.log`, release reason now includes how
  long the peer had been silent, `session.log` lines no longer interleave mid-line,
  and `system.txt` includes Wi-Fi interface/BSSID/signal and adapter power settings.
- Version 1.1.0 across the app, service and installer.

## If you still see stalls

Wi-Fi power saving and roaming are the usual culprits. On the client:

```powershell
Set-NetAdapterPowerManagement -Name "Wi-Fi" -AllowComputerToTurnOffDevice Disabled
netsh wlan show interfaces          # prefer 5 GHz and a high rate
powercfg /setactive SCHEME_MIN
```

Driver settings: Power Save = CAM / Maximum Performance, Roaming Aggressiveness = lowest.
Ethernet on the client is still the single biggest improvement.

---

# 繁體中文

可靠性修正版：減少卡頓、避免控制突然跳回。

## 下載

| 檔案 | 大小 | SHA-256 |
|---|---|---|
| `LAN-KVM-Setup-1.1.0.exe` | 2.96 MB | `EABF52D0671A5241A0626CDA8C73C94A2CB3AA3F436D6B0B408D89C74E2CF712` |

## 修改內容

- **存活判定 3 秒 → 10 秒**，且**任何封包**都算活著；client 另外每 500 ms 主動 keepalive。
  Wi-Fi 停頓 1–4 秒不會再被強制放開。
- **滑鼠封包合併**：master 每 4 ms 最多送一包（250 Hz），client 把積壓的 motion 合成一筆。
  停頓後不再「慢半拍後爆衝」。
- **回切改成明確動作**：`release_px` 預設 2 → 10，且要連續 2 個向外封包才回切。
- **IP 變更不再拆掉工作中的連線**：忽略 169.254／loopback／multicast，新位址要連續 2 次公告，
  切換前先探測舊位址。
- **健康檢查去抖**：服務端 6 次失敗（30 秒）才重啟 agent；App 端 3 次（15 秒）才翻狀態。
- **UDP 緩衝區加大到 1 MB**。
- **log**：`engine.log` 每 5 秒 `[net] tx/rx/rtt/rtt_max/silent`；client `session.log` 每 5 秒
  `[net] rx/max_gap`；釋放原因附上靜默時間；`session.log` 不再行內交錯；
  `system.txt` 加入 Wi-Fi 介面／BSSID／訊號與網卡省電設定。

## 若還是會卡

先處理 Wi-Fi 省電與漫遊（client 端）：

```powershell
Set-NetAdapterPowerManagement -Name "Wi-Fi" -AllowComputerToTurnOffDevice Disabled
netsh wlan show interfaces
powercfg /setactive SCHEME_MIN
```

驅動：Power Save = CAM / Maximum Performance、Roaming Aggressiveness = 最低。
client 改有線仍是效果最大的解。
