# LAN-KVM 1.1.9

> Released 2026-10-30 · English | [繁體中文](#繁體中文)

## English

### Summary

1.1.9 fixes a crash that happened on **every Windows boot**. The service underflowed a
monotonic clock while scheduling its first session-agent launch, panicked, and needed two
SCM restarts plus about 30 seconds before the client engine came up.

All 1.1.8 features (log rotation, streamed log bundles, log sizes and **Clear logs**) are
unchanged.

### Symptom

`C:\ProgramData\LAN-KVM\logs\service.log` after every boot:

```
[1793322225] [4780] PANIC: panicked at library\std\src\time.rs:430:33:
overflow when subtracting duration from instant
```

- SCM restarted the service twice; `service started` only succeeded ~30 s later
- The first auto-connect could fail (`session_start ignored: SYSTEM service is not running`)
- `sc query LAN-KVM` could report `STOPPED` / `START_PENDING` during that window

### Root cause

`crates/lankvm-service/src/service.rs` back-dated `Instant` to make the first launch
attempt due immediately:

```rust
let mut last_attempt = Instant::now() - Duration::from_secs(30);
```

`Instant` on Windows counts from boot. On a machine that has been up for less than 30
seconds the subtraction underflows and Rust panics. The service is configured to start at
boot, so this happened on every reboot. Line 184 used the same pattern with 5 seconds.

### Fix

- `last_attempt` is now `Option<Instant>`: `None` = never attempted → launch immediately
- The health-check failure path resets it to `None` instead of back-dating it
- New `retry_due(last, interval)` helper with unit tests
- The retry interval (`LAUNCH_RETRY`, 5 s) is unchanged
- Side effect: the session agent now starts immediately after boot instead of waiting 30 s

### Files

```
LAN-KVM-Setup-1.1.9.exe
size   2,887,417 bytes
SHA256 3B358B6A3D9639A23EBEA56D63E5947539F5DEE12A9267DF1F4B6A7001442A48
```

### Verification

- [ ] `sc query LAN-KVM` is `RUNNING` within 5 s of boot
- [ ] `service.log` has no `PANIC` / `overflow when subtracting duration`
- [ ] Auto-connect works right after boot, without the old 30 s wait
- [ ] Stopping the session agent (or 6 failed health probes) restarts it immediately
- [ ] Log rotation, log collection and Clear logs behave exactly like 1.1.8
- [ ] Edge switching, sign-in screen control and clipboard are unchanged

### Notes

- Only the service startup scheduling changed: no protocol, input-path or log-format changes.
- The 1.1.7 data-port fix and all 1.1.8 log limits are retained.

---

## 繁體中文

> 重點：修正**每次開機時 service 會 panic**，導致 client engine 要等約 30 秒才起來。
> 1.1.8 的紀錄輪替、bundle 串流與清除紀錄全部保留。

### 1. 問題現象

每次 Windows 開機後，`C:\ProgramData\LAN-KVM\logs\service.log` 會出現：

```
[1793322225] [4780] PANIC: panicked at library\std\src\time.rs:430:33:
overflow when subtracting duration from instant
```

- SCM 會重啟 service 兩次，約 30 秒後才真正 `service started`
- 開機後第一次自動連線可能失敗，app.log 出現
  `session_start ignored: SYSTEM service is not running`
- 這段時間 `sc query LAN-KVM` 可能是 `STOPPED` / `START_PENDING`

### 2. 根因

`crates/lankvm-service/src/service.rs` 用「把時間往回撥」的方式讓第一次啟動嘗試立即發生：

```rust
let mut last_attempt = Instant::now() - Duration::from_secs(30);
```

Windows 的 `Instant` 是開機後的單調計數，開機未滿 30 秒時這個相減會 underflow，
Rust 直接 panic（`overflow when subtracting duration from instant`）。
service 是開機自動啟動，所以**每次開機**都會踩到；約 30 秒後才不會再發生。

第 184 行也有同樣寫法（`Instant::now() - 5 秒`），開機未滿 5 秒時同理。

### 3. 修正內容

- `last_attempt` 改成 `Option<Instant>`：
  - `None` = 從未嘗試過 → 立即執行（正是開機時要的行為）
  - `Some(t)` = 上次嘗試時間 → 間隔滿 `LAUNCH_RETRY`（5 秒）才再試
- 健康檢查失敗要「立刻重試」時，改成設回 `None`，不再做時間相減
- 新增 `retry_due(last, interval)` 與單元測試：
  - `retry_is_due_immediately_when_never_attempted`
  - `retry_waits_for_the_interval`
- 副作用是好的：開機後 session agent 立即啟動，不用再等 30 秒

### 4. 檔案

```
dist-app\LAN-KVM-Setup-1.1.9.exe
size   2,887,417 bytes
SHA256 3B358B6A3D9639A23EBEA56D63E5947539F5DEE12A9267DF1F4B6A7001442A48
```

建置：

```powershell
cargo build --release -p lankvm-app -p lankvm-service
installer\build.cmd
```

### 5. 驗收清單

- [ ] 重開機後 5 秒內 `sc query LAN-KVM` 就是 `RUNNING`
- [ ] `service.log` 不再出現 `PANIC` / `overflow when subtracting duration`
- [ ] 開機後自動連線成功，不需要等 30 秒
- [ ] 讓 session agent 停止（或健康檢查連續失敗）後，service 會立即重新啟動它
- [ ] 1.1.8 的紀錄輪替、收集紀錄、清除紀錄行為不變
- [ ] 邊界切換、登入畫面、剪貼簿與 1.1.8 相同

### 6. 備註

- 這一版只動 service 的啟動排程，沒有改動協定、輸入路徑或 log 格式。
- 1.1.7 的 data-port 修正（預設 45872、舊 `client_port=45871` 遷移、安裝程式覆寫 config）
  與 1.1.8 的紀錄上限全部保留。
