# LAN-KVM 1.1.10

> Released 2026-11-05 · English | [繁體中文](#繁體中文)

## English

### Summary

Fixes edge switching going dead after a fullscreen app, a resolution switch or a monitor change,
and adds diagnostics for the remaining "I pushed at the edge and nothing happened" cases.

### Symptom

After opening an app whose window fills the screen width, pushing the cursor through the
configured edge did nothing. Clicking the taskbar once made it work again. Reported on 1.1.9.

### What the collected logs showed

From the 2026-11-05 bundle (master = laptop, client = desktop):

```
11:31:00  [master] remote control OFF (edge, peer silent 1 ms)
11:33:26  [master] remote control ON  (Right edge, entry ratio 43062, cursor hider on)
```

- The master produced **no `remote control ON` for 2m26s** while the user was pushing at the edge
- Heartbeats continued (`rtt=1–2 ms`), so the network and the peer were fine
- The client stayed `controlled=false` the whole time
- Conclusion: the master's edge **detector never fired**; this was not a release/bounce problem

### Root cause

`crates/lankvm-platform/src/poc.rs` captured the virtual-desktop bounds **once** when the engine
started and reused them forever:

```rust
let bounds = win::virtual_bounds()?;
let _ = BOUNDS.set(bounds);      // OnceLock, never refreshed
```

The trigger requires the cursor to reach `bounds.right - 1`. Any display change after the engine
starts — an exclusive-fullscreen app, a resolution switch, a monitor being added/removed — makes
that coordinate unreachable, so pushing at the real screen edge silently does nothing until the
mode returns to what it was when the engine started (which is what clicking the taskbar often
does).

### Fix

- Read `virtual_bounds()` live on every raw input event (four cheap `GetSystemMetrics` calls)
- Track the bounds in the heartbeat and log `[master] display bounds changed -> ...`, resetting
  the edge accumulator on a change
- `do_release` also warps the cursor using live bounds

### Diagnostics (for any remaining cases)

- `[master] edge push blocked edge=Right cursor=(1919, 507) pushed=140px acc=18px bounds=... clip=... fg=... lmb=false`
  — rate-limited to once per 3 s, only when the pointer is on the edge and 80 px of outward push
  did not trigger. `clip` shows a foreground app confining the pointer, `fg` names that app,
  `lmb` shows a stuck/held button.
- `[net] ... raw=NNN/s` — raw mouse events per second, so a log can distinguish "input stopped"
  from "edge test failed".

### Files

```
LAN-KVM-Setup-1.1.10.exe
size   2,888,936 bytes
SHA256 7C7016D73D2D898910982CEC32070481588270AF9390E2A184620E16FCB00461
```

### Verification

- [ ] Open a fullscreen / maximised app, then push through the configured edge — it switches
- [ ] `engine.log` shows `display bounds changed` when a fullscreen app changes the mode
- [ ] Normal edge switching still works with no extra log lines
- [ ] If it still fails, `engine.log` contains an `edge push blocked` line with the cause
- [ ] Clipboard, sign-in screen and Num Lock unchanged

### Notes

- No protocol or input-format changes.
- 1.1.9 (boot panic) and 1.1.8 (log rotation/bundles/Clear logs) are included.

---

## 繁體中文

> 重點：修正「全螢幕程式或解析度切換後，推邊界完全沒反應」，並加上診斷 log。

### 1. 現象

開啟一個寬度貼齊螢幕的程式後，游標推到設定的邊界沒有反應，要點一下下方工具列才會恢復
（1.1.9 回報）。

### 2. Log 分析（2026-11-05 bundle，master = 筆電）

```
11:31:00  [master] remote control OFF (edge, peer silent 1 ms)
11:33:26  [master] remote control ON  (Right edge, entry ratio 43062, cursor hider on)
```

- 中間 **2 分 26 秒沒有任何 `remote control ON`**，代表 master 的邊界偵測完全沒觸發
- 心跳持續、`rtt=1–2 ms`，網路與對端都正常
- client 全程 `controlled=false`
- 所以不是「進去了又被彈回來」，是**根本沒進去**

### 3. 根因

`crates/lankvm-platform/src/poc.rs` 只在 engine 啟動時抓一次虛擬桌面範圍，之後永遠沿用：

```rust
let bounds = win::virtual_bounds()?;
let _ = BOUNDS.set(bounds);      // OnceLock，永不更新
```

觸發條件要求游標到達 `bounds.right - 1`。只要 engine 啟動後顯示模式改變（全螢幕程式、
解析度切換、螢幕增減），這個座標就到不了，推真正的螢幕邊界也會完全沒反應；等模式回到
啟動時的樣子（通常就是點工具列離開全螢幕）才會恢復。

### 4. 修正

- 每個 raw input 事件都即時讀 `virtual_bounds()`（4 次 `GetSystemMetrics`，成本可忽略）
- 心跳每秒比對範圍，變更時寫 `[master] display bounds changed -> ...` 並重設邊界累計
- `do_release` 也用即時範圍換算游標位置

### 5. 診斷（若還有其他原因）

- `[master] edge push blocked ...`：只在游標真的貼在邊界、且外推超過 80 px 仍未觸發時记录，
  每 3 秒最多一筆。內容包含游標座標、即時範圍、`clip`（被前景程式限制）、`fg`（前景視窗
  類別）、`lmb`（左鍵是否被按住）與累計值。
- `[net] ... raw=NNN/s`：每秒 raw input 事件數，可區分「input 斷了」與「邊界判斷錯誤」。

### 6. 檔案

```
dist-app\LAN-KVM-Setup-1.1.10.exe
size   2,888,936 bytes
SHA256 7C7016D73D2D898910982CEC32070481588270AF9390E2A184620E16FCB00461
```

### 7. 驗收清單

- [ ] 開全螢幕／最大化程式後，推邊界仍可切換
- [ ] 全螢幕造成模式改變時，`engine.log` 出現 `display bounds changed`
- [ ] 正常使用時不會多出 `edge push blocked` 之類的 log
- [ ] 若仍失敗，`engine.log` 會有 `edge push blocked` 直接指出原因
- [ ] 剪貼簿、登入畫面、Num Lock 行為不變

### 8. 備註

- 沒有改動協定或輸入格式。
- 1.1.9（開機 panic）與 1.1.8（紀錄輪替／bundle／清除紀錄）都包含在內。
