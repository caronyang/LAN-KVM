# LAN-KVM 1.1.12

> Released 2026-11-05 · English | [繁體中文](#繁體中文)

## English

### What the 1.1.11 logs proved

Every hand-off in the 1.1.11 test matched exactly (clock skew ~16 s between machines):

```
client y=816 (75.6%) -> master b=52719, flags=0x100 -> returned y=816 (75.6%)
client y=868 (80.4%) -> master b=52719 ...            -> returned y=868 (80.4%)
client y=151 (14.0%) -> master b=9171  ...            -> returned y=151 (14.0%)
client y=872 (80.8%) -> master b=52962 ...            -> returned y=872 (80.8%)
client y=884 (81.9%) -> master b=53691 ...            -> returned y=884 (81.9%)
client y=138 (12.8%) -> master b=8381  ...            -> returned y=138 (12.8%)
client y=896 (83.0%) -> master b=54420 ...            -> returned y=896 (83.0%)
client y=146 (13.5%) -> master b=8867  ...            -> returned y=146 (13.5%)
```

8/8 exact, every packet had `b` and `flags=0x100`, and none of the 53 returns in the log
used the old entry height. So the intended return point was already correct.

### What this build changes

If the *intended* point was right but the user still saw the old height, the pointer was
visible at the parked edge position before `SetCursorPos` moved it. `do_release` used to
show the cursor first and warp afterwards; it now warps first and only then shows the cursor,
so it never appears at the entry height and jumps.

The return log now also records the actual cursor position after the warp:

```
[master] cursor returned at y=868 (client exit height 80.4%) at=Some(Point { x: 1880, y: 868 })
```

If `at` ever differs from `y`, `SetCursorPos` was blocked or clamped and we will see it
directly.

### Files

```
LAN-KVM-Setup-1.1.12.exe
size   2,890,176 bytes
SHA256 A76C41E90CF1C13C739817A5AF7E70C6355D932964C4AC112AD783412941E309
```

### Test

1. Install 1.1.12 on both machines.
2. Enter at the top → leave at the bottom, and the reverse, several times.
3. Collect logs with the peer online and check `at=` in `engine.log`.

---

## 繁體中文

### 1.1.11 的 log 證明了什麼

1.1.11 的測試中，每次交班都完全吻合（兩台時鐘差約 16 秒）：

```
client y=816（75.6%）→ master b=52719、flags=0x100 → 回到 y=816
client y=868（80.4%）→ 回到 y=868
client y=151（14.0%）→ 回到 y=151
client y=872（80.8%）→ 回到 y=872
client y=884（81.9%）→ 回到 y=884
client y=138（12.8%）→ 回到 y=138
client y=896（83.0%）→ 回到 y=896
client y=146（13.5%）→ 回到 y=146
```

8/8 精確吻合，每個封包都有 `b` 與 `flags=0x100`；整份 log 的 53 次回程**沒有一次**回到
舊的進入高度。也就是說「要回到哪裡」這件事本身已經是正確的。

### 這一版改了什麼

如果目標位置正確、但視覺上仍看到舊高度，最可能是游標在 `SetCursorPos` 之前就先被顯示在
被釘住的位置。`do_release` 原本是「先顯示游標、再 warp」，現在改成「先 warp、最後才顯示」，
所以不會先出現在進入高度再跳走。

回程 log 也加上了 warp 後實際的游標位置：

```
[master] cursor returned at y=868 (client exit height 80.4%) at=Some(Point { x: 1880, y: 868 })
```

若 `at` 與 `y` 不同，就代表 `SetCursorPos` 被限制或失敗，可以直接看出來。

### 檔案

```
dist-app\LAN-KVM-Setup-1.1.12.exe
size   2,890,176 bytes
SHA256 A76C41E90CF1C13C739817A5AF7E70C6355D932964C4AC112AD783412941E309
```

### 測試

1. 兩台都安裝 1.1.12
2. 上進 → 下回、下進 → 上回，各測幾次
3. 收集紀錄（對方要在線上），確認 `engine.log` 的 `at=` 與 `y=` 相同
