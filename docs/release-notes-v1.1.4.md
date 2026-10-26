# LAN-KVM 1.1.4

Reliability release: fewer stalls, no surprise control hand-backs, a fix for the
1.1.0 keepalive regression, and a fix for `Shift + /` on the controlled machine.

## Download

| File | Size | SHA-256 |
|---|---|---|
| `LAN-KVM-Setup-1.1.4.exe` | 2.96 MB | `B9780DDD6704554C2568E4EAF5A0CE016412275EBD67740E8C22B30ED6CB595B` |

## What changed

**Networking / feel**

- **10 s liveness window** (was 3 s) and **any packet counts as alive**, including a
  client keepalive. A 1–4 s Wi-Fi stall no longer hands control back.
- **Frame-coalesced mouse motion** on both ends: the master sends at most 250 packets/s
  and the client merges a backlog into one relative move.
- **Edge return is deliberate**: `release_px` default 2 → 10 and two consecutive outward
  packets at the edge are required.
- **IP changes no longer drop a working session**: link-local/loopback/multicast sources
  are ignored, a new address needs two announcements, and the old address is probed
  before reconnecting.
- **Debounced health checks**: 6 failed probes (30 s) before the service restarts the
  agent; 3 misses (15 s) before the app flips state.
- **1 MB UDP buffers** on both sides.

**1.1.1 hotfix (keepalive)**

- Keepalives are only sent after real input and stop after 5 s, so they never target a
  closed probe/engine port.
- `WSAECONNRESET` / `WSAETIMEDOUT` on UDP receive (ICMP port-unreachable) is treated as
  a lost datagram instead of killing the listener.
- If the client listener really dies, the agent exits so the service relaunches it in 5 s.

**1.1.2 keyboard fix**

- Every key packet now carries the master's Shift state; the client re-asserts it before
  the key. A single lost Shift packet no longer turns `Shift + /` into `/`.
- Shift is driven through one scan code, so left/right Shift can never leave a
  half-pressed modifier; stray extended flags on Shift are ignored.
- Throttled `session.log` lines report a Shift resync or an injected numpad `/`.

**1.1.3 emergency / stuck cursor**

- UDP sends now have a 100 ms timeout, so a vanished peer or an exhausted socket
  buffer can no longer freeze the heartbeat thread.
- An independent watchdog releases control after 10 s of silence; if the release
  cannot be processed within 5 s it terminates the engine, which makes Windows
  drop ClipCursor, the cursor-hider window and the hidden-cursor count.
- **New global hotkey `Ctrl+Alt+Shift+R`** (registered by the app, so it works even
  if the engine is wedged): force-stop the session and take the mouse back.
  Also available in the tray menu, the UI (red button) and `POST /api/emergency`.
- Quitting the app now releases the session first.
- The client releases held keys after 15 s without master traffic.
- `engine.log` is appended (with a start marker) instead of overwritten; rotates at 8 MB.

**1.1.4 Shift regression fix**

- The master now tracks the physical Shift state itself. Returning 1 from the
  low-level hook stops Windows from updating the async key state, so the old
  `GetAsyncKeyState(VK_SHIFT)` lookup always reported "up".
- The modifier flag now has a validity bit. Peers that do not send it (1.1.2 and
  older) are ignored instead of being read as "Shift released", so mixed versions
  keep typing `?` correctly.
- The engine also accepts `Ctrl+Alt+Shift+R`, not just `Q`.
- When the client sees 15 s of silence it releases keys *and* tells the master,
  so a recovered master does not keep sending input into a deaf client.

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

可靠性修正版：減少卡頓、避免控制突然跳回、修正 keepalive 造成的反覆重啟，
以及被控端 `Shift + /` 只出現 `/` 的問題。

## 下載

| 檔案 | 大小 | SHA-256 |
|---|---|---|
| `LAN-KVM-Setup-1.1.4.exe` | 2.96 MB | `B9780DDD6704554C2568E4EAF5A0CE016412275EBD67740E8C22B30ED6CB595B` |

## 修改內容

**連線與手感**

- **存活判定 3 秒 → 10 秒**，任何封包都算活著；Wi-Fi 停頓 1–4 秒不會再被強制放開。
- **滑鼠封包合併**：master 每 4 ms 最多一包，client 把積壓的 motion 合成一筆。
- **回切要明確**：`release_px` 預設 10，且需連續 2 個向外封包。
- **IP 變更不再拆連線**：忽略 APIPA/loopback/multicast，新位址要連續 2 次公告，
  切換前先探測舊位址。
- **健康檢查去抖**：服務端 6 次失敗（30 秒）、App 端 3 次（15 秒）才翻狀態。
- UDP 緩衝區 1 MB。

**1.1.1 熱修（keepalive）**

- keepalive 只在收到真正的輸入封包後啟動，且 5 秒沒新輸入就停止。
- UDP 收到 `WSAECONNRESET / WSAETIMEDOUT`（ICMP port unreachable）視為掉包。
- listener 真的死亡時 agent 直接退出，服務 5 秒內重啟。

**1.1.2 鍵盤修正**

- 每個 Key 封包都帶 master 目前的 **Shift 狀態**；client 在注入按鍵前同步，
  單一 Shift 封包遺失不會再把 `Shift + /` 變成 `/`。
- Shift 一律用同一個 scan code 驅動，左右 Shift 不會再出現半按狀態；
  Shift 的 extended 旗標一律忽略。
- `session.log` 會（每 5 秒最多一行）記錄 Shift 重新同步或收到 numpad `/`。

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
