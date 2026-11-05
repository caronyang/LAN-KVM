# LAN-KVM 1.1.11

> Released 2026-11-05 · English | [繁體中文](#繁體中文)

## English

### Summary

Diagnostic build for the pointer return height. Behaviour is unchanged from 1.1.10; this
release only adds log lines on both ends so a bundle shows exactly what each side saw.

### Why

On 1.1.10 the pointer usually returns at the height it left the peer, but sometimes it
returns at the height it originally entered at. The user's reproduction is precise:

```
enter at the top    -> leave at the bottom -> pointer comes back at the top
enter at the bottom -> leave at the top    -> pointer comes back at the bottom
```

That is exactly what happens when the master falls back to its parked cursor (no exit
height in the Release packet), so the next step is to log both ends of that hand-off.

### What is logged now

Client (`session.log`, SYSTEM service agent):

```
switch: controlled=true entry_edge=1 ratio=52428 bounds=Some(Rect { left: 0, top: 0, right: 1920, bottom: 1080 })
Default: inject Switch entry_edge=1 ratio=52428 -> (0, 863)
cursor at entry edge -> release (virtual_x=-42, edge=1, y=1042, ratio=63210)
```

Master (`engine.log`):

```
[net] release reason=5 b=63210 flags=0x100
[master] cursor returned at y=1052 (client exit height 96.4%)
```

If the master prints `b=-1` or `flags=0x0`, the client did not send a height.
If it prints a correct `b`/`flags` but a different `y` than the client's `y`, the bug is in
the mapping or the warp.

### Files

```
LAN-KVM-Setup-1.1.11.exe
size   2,890,464 bytes
SHA256 6084E894B384788478AA58AB3F38F094697A4C3ACF9540FBD46D2AEAA0142D38
```

### How to test

1. Install 1.1.11 on **both** machines (version must match).
2. Enter the peer at the top, move to the bottom, leave at the bottom. Repeat in reverse.
3. Collect logs **while the peer is online** (the previous bundle only had this side).
4. Send the folder.

### Notes

- No behaviour, protocol or UI change beyond the new log lines.

---

## 繁體中文

> 重點：回程高度的診斷版。行為與 1.1.10 相同，只增加兩端的 log，讓 bundle 能看出
> 「B 實際的位置」與「A 套用的位置」。

### 為什麼要加

1.1.10 大多時候回程高度正確，但有時候會回到「當初進入的高度」。你的描述很明確：

```
上進 → 下回 → 結果回到上方（進入點）
下進 → 上回 → 結果回到下方（進入點）
```

這正是 master 拿不到 exit 高度、退回原本被釘住的游標位置時的行為，所以下一步是把
兩端的交班過程都記錄下來。

### 現在會記錄什麼

Client（`session.log`，SYSTEM service agent）：

```
switch: controlled=true entry_edge=1 ratio=52428 bounds=...
Default: inject Switch entry_edge=1 ratio=52428 -> (0, 863)
cursor at entry edge -> release (virtual_x=-42, edge=1, y=1042, ratio=63210)
```

Master（`engine.log`）：

```
[net] release reason=5 b=63210 flags=0x100
[master] cursor returned at y=1052 (client exit height 96.4%)
```

判讀方式：

- master 出現 `b=-1` 或 `flags=0x0` → client 沒有送出高度
- `b`/`flags` 正確，但 master 的 `y` 與 client 的 `y` 不同 → 問題在換算或 warp

### 檔案

```
dist-app\LAN-KVM-Setup-1.1.11.exe
size   2,890,464 bytes
SHA256 6084E894B384788478AA58AB3F38F094697A4C3ACF9540FBD46D2AEAA0142D38
```

### 測試方式

1. 兩台都安裝 1.1.11（版本要一致）
2. 上進 → 移到下方 → 下回；再反過來測一次
3. 收集紀錄時**對方要在線上**（上一次的 bundle 只有本機端，沒有 master 的 engine.log）
4. 把整個資料夾給我

### 備註

- 除了新增 log，沒有行為、協定或 UI 變更。
