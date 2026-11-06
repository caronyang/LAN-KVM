# LAN-KVM 1.1.13

> Released 2026-11-06 · English | [繁體中文](#繁體中文)

## English

### The bug

On 1.1.12, after working on the peer for a while, pushing at the edge to come back often did
nothing. It crossed only after many attempts, and then worked normally for a short time.

The client decided to hand control back with `virtual_x`, a running sum of every mouse delta
received since it took control. That sum is not the pointer:

- Windows applies pointer acceleration to injected motion, so raw counts and screen pixels are
  not 1:1.
- At a screen edge the cursor stops but the deltas keep arriving, so the sum keeps growing while
  the pointer is pinned.

After some use the sum had drifted, usually positive after touching the far edge. Returning
required the sum to fall below `-release_px`. A push at the entry edge smaller than the drift did
nothing, and moving back to try again added the same amount back, so **every retry started from
the same value**. Only a single push larger than the drift could cross — exactly the "try many
times, eventually it works" behaviour.

The logs confirm the hand-off itself was never the problem: all 78 releases in the last bundle
carried `b` and `flags=0x100`, and the return height matched on both ends.

### The fix

`crates/lankvm-input/src/lib.rs` now has a `ReturnPush` tracker used by the service client:

- it counts only outward movement while the pointer is **actually sitting on the entry edge**;
- any inward movement, or the pointer leaving the edge, clears it;
- it still requires two consecutive outward packets and `release_px` (10) in total.

The history of the session can no longer block the return, and a retry always starts from zero.
Four unit tests cover the drift, retry, inward and vertical-motion cases.

The release log now shows the push instead of the old sum:

```
cursor at entry edge -> release (push=24px cursor=Some(Point { x: 0, y: 812 }), edge=1, y=812, ratio=49296)
```

### Files

```
LAN-KVM-Setup-1.1.13.exe
size   2,891,004 bytes
SHA256 030ECAED1D9CCA3B185D71F8D2B2F6052D1706FED19C41E68FC68302E263F946
```

### How to test

1. Install 1.1.13 on **both** machines (versions must match).
2. On the peer, work for several minutes: use all four corners, push into the far edge, drag
   windows against the edges.
3. Push back across the entry edge — it should cross on the **first** deliberate push, every
   time, including immediately after entering.
4. Repeat 10+ times and collect a log bundle with the peer online.

---

## 繁體中文

### 問題

1.1.12 在被控端操作一段時間後，推邊界常常回不去，要試很多次才偶爾成功；成功後短時間內又正常。

原因是 client 用 `virtual_x` 決定何時交還控制：那是「取得控制後所有滑鼠位移的累加總和」。
這個總和並不等於游標位置：

- Windows 對注入的移動會套用滑鼠加速，raw counts 與螢幕像素不是 1:1。
- 游標撞到螢幕邊緣後就停住，但位移仍持續進來，總和會在游標被釘住時繼續增加。

使用一段時間後總和會偏移（通常是偏向正值，尤其在碰過另一側邊緣之後）。回程要求總和降到
`-release_px` 以下。若在進入邊界的推力小於偏移量，就完全沒反應；退回重試時又把同樣的量加
回去，所以**每次重試都從同一個值開始**。只有單次推力大於偏移量才會過關——這就是「試很多次
才偶爾成功」的原因。

log 也證實交班本身沒問題：上一份 bundle 的 78 次回程全部帶著 `b` 與 `flags=0x100`，兩端高度
也一致。

### 修正

`crates/lankvm-input/src/lib.rs` 新增 `ReturnPush`，service client 改用它：

- 只累計「游標真的貼在進入邊界時」的向外位移；
- 任何向內移動、或游標離開邊界，就歸零；
- 仍然要求連續 2 個向外封包、且總量達到 `release_px`（10）。

整個 session 的歷史不再能卡住回程，每次重試都從零開始。新增 4 個單元測試涵蓋偏移、重試、
向內與垂直移動。

回程 log 改為記錄推力，而不是舊的總和：

```
cursor at entry edge -> release (push=24px cursor=Some(Point { x: 0, y: 812 }), edge=1, y=812, ratio=49296)
```

### 檔案

```
dist-app\LAN-KVM-Setup-1.1.13.exe
size   2,891,004 bytes
SHA256 030ECAED1D9CCA3B185D71F8D2B2F6052D1706FED19C41E68FC68302E263F946
```

### 測試方式

1. 兩台都安裝 1.1.13（版本要一致）。
2. 在被控端操作數分鐘：用過四個角落、推過另一側邊緣、把視窗拖到邊界。
3. 推回進入邊界——應該**第一次**用力推就回去，包含剛進入後立刻推回。
4. 重複 10 次以上，並在對方在線時收集 log bundle。
