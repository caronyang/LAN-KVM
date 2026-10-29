# LAN-KVM 1.1.7

> 發佈日期：2026-10-29  
> 重點：修正升級後 SYSTEM client engine 沒有在 **data port 45872** 監聽，導致 app 顯示「client 引擎未就緒」、`session_start` 被忽略。

## 1. 問題現象

從 1.1.6 或更舊版本升級後：

- app 顯示「client 引擎（SYSTEM 服務）未就緒」
- app log 每 6 秒出現：

```
session_start ignored: SYSTEM service is not running
```

- `sc query LAN-KVM` 顯示 service 是 `RUNNING`
- `session.log` 有 hook worker，但沒有：

```
P0 UDP client listening on 0.0.0.0:45872
```

- `state.json` 的 `"service_running"` 一直是 `false`
- 配對、剪貼簿正常，但滑鼠無法穿越

## 2. 根因

1. app 的健康檢查是送 P0 Ping 到 `127.0.0.1:45872`，並要求 `Pong` 的 `d=1`。
2. SYSTEM service 只有在讀到 `C:\ProgramData\LAN-KVM\config.txt` 的 `client_port` 時，才會帶 `--client --port` 啟動 session agent。
3. 1.1.6 的 `Config::default()` 是 `client_port: None`：
   - config 不存在、讀取失敗、或寫入失敗 → 完全沒有 45872 listener
4. 舊版留下的 config 是：

```
client_port=45871
```

`45871` 是 app 的 **control port**，不是 data port。service 會去搶 app 的 control port，造成 app bind 失敗；即使不搶，app 的 45872 健康檢查也不會成功。

5. Inno Setup 的 `[Files]` 使用：

```
Flags: onlyifdoesntexist
```

升級時不會覆寫舊 config，所以舊的 `45871` 一直留著。

## 3. 修正內容

### Service

- `Config::default()` 改為 `client_port = 45872`。
- `config.txt` 不存在 → 使用 45872。
- 舊值 `45871`、`0`、無效值 → 自動遷移成 45872。
- 保留 `release_px` 預設 10。

### Installer

- `installer/lankvm.iss` 的 config 改成 **升級時覆寫**：

```
Source: "config.txt"; DestDir: "{commonappdata}\LAN-KVM"; Flags: ignoreversion
```

- `installer/config.txt` 內容為：

```
client_port=45872
release_px=10
```

- 自解壓版 setup 會把 config 寫入結果記錄到 `%TEMP%\lankvm-setup.log`，不再靜默失敗。

### 其他

- 修正 `cargo clippy` 的 raw pointer 警告，release build 可正常完成。

## 4. 檔案

```
dist-app\LAN-KVM-Setup-1.1.7.exe
size   2,871,742 bytes
SHA256 D057EA72E1D0DAB5C7C66D64E9B196C1DB1BC55631E67DC0F153ECD374B8C9C0
```

建置：

```powershell
cargo build --release -p lankvm-app -p lankvm-service
installer\build.cmd
```

## 5. 升級注意

- 直接執行 1.1.7 安裝程式即可；會停止舊 app/service、覆寫 config、更新 service。
- 升級後確認：

```powershell
sc.exe query LAN-KVM
Get-Content C:\ProgramData\LAN-KVM\config.txt
Get-NetUDPEndpoint -LocalPort 45872
```

正確狀態：

- `LAN-KVM` → `RUNNING`
- `config.txt` → `client_port=45872`
- `45872` → `lankvm-service` session agent
- `45871` → `lankvm-app`

- 若之前手動把 config 改名，重新安裝會重建 45872；不需要手動處理。
- 不建議再用 `client_port=45871`；那是舊版錯誤值。

## 6. 驗收清單

- [ ] 乾淨安裝：app 啟動後 5 秒內顯示 client engine ready
- [ ] 從 1.1.6 升級（故意保留 `client_port=45871`）：升級後 config 變成 45872
- [ ] `session.log` 出現 `P0 UDP client listening on 0.0.0.0:45872`
- [ ] app log 不再出現 `session_start ignored: SYSTEM service is not running`
- [ ] A 推邊界 → B 滑鼠可穿越
- [ ] B 鎖定/登入畫面 → Winlogon injector 可用
- [ ] 解除配對/重新配對正常
- [ ] 卸載後 service、防火牆規則、config 清理正常

## 7. 備註

- 1.1.6 的延遲、重連、鍵盤、緊急釋放修正全部保留。
- 本版沒有改動 P0 未加密協定；QUIC/TLS/SPAKE2 仍在下一個里程碑。
- 若 Windows 防火牆或 Apex One 擋住 UDP 45872，app 仍會顯示 service not ready；這次先確認 `Get-NetUDPEndpoint -LocalPort 45872` 是否有 listener。
