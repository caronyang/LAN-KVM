LAN-KVM 0.2（測試版）
=====================

需求
----
- Windows 10 / 11 x64
- 兩台電腦在同一個區域網路（同一個 AP / Switch）
- 兩台都安裝本程式

安裝
----
1. 解壓縮這個資料夾（setup.exe 與兩個 exe 必須在一起）。
2. 執行 setup.exe（會跳 UAC，按「是」）。
3. 安裝到 C:\Program Files\LAN-KVM，並自動：
   - 建立防火牆規則：UDP 45870 / 45871 / 45872（不需要再手動加）
   - 安裝並啟動 LAN-KVM SYSTEM 服務（登入畫面 / UAC 控制）
   - 設定開機自動啟動（--silent）
   - 建立桌面與開始功能表的 LAN-KVM 捷徑
   - 移除舊版 LAN-KVM 服務並換成新檔
4. 瀏覽器會自動開啟設定頁面。

第一次配對
----------
1. 在 A 電腦開啟 LAN-KVM（桌面捷徑）。
2. 「掃描到的電腦」會列出同網段的另一台，按「配對」。
3. B 電腦會自動開啟 UI，顯示 3 位數配對碼。
4. 在 A 輸入該號碼，按確定。
5. 配對成功後會自動連線；之後兩台開機都會自動連線，直到解除配對。

切換
----
- 在 A 的 UI 設定「對方螢幕在我的右邊/左邊」。
- A 的滑鼠推過設定的邊界 → 控制 B。
- 把游標推回 B 的進入邊界（越界 2px）→ 控制權回到 A。
- B 自己的滑鼠/鍵盤不會改變連線狀態（設計如此）。
- 門檻可調：切入推力 px、時間窗 ms、回切越界 px。

登入畫面 / UAC
--------------
- B 的 client 注入由 SYSTEM 服務執行，所以 B 按 Win+L 或跳 UAC 時，
  仍然可以用 A 的鍵鼠操作。
- 若 UI 顯示「SYSTEM 服務未就緒」，請確認：
  sc query LAN-KVM
  服務必須是 RUNNING；必要時以管理員執行 sc start LAN-KVM。

收集測試紀錄
------------
- 測試完在兩台的 UI 右上按「收集紀錄」。
- 會開啟一個資料夾（桌面或使用者目錄下）：
  LAN-KVM-logs-<電腦名>-<時間>
- 內含：
  app.log / engine.log / app.json / state.json
  service.log / session.log（登入畫面相關）
  system.txt（ipconfig、服務狀態、UDP、防火牆規則）
  setup.log（安裝記錄）
- 把兩台的整個資料夾交給開發者分析即可。

設定與資料
----------
- 配對、名稱、門檻：%LOCALAPPDATA%\LAN-KVM\app.json
- 程式日誌：%LOCALAPPDATA%\LAN-KVM\app.log
- 連線日誌：%LOCALAPPDATA%\LAN-KVM\engine.log
- 服務日誌：C:\ProgramData\LAN-KVM\logs\service.log、session.log

解除配對 / 停止
---------------
- UI 的「解除」：兩邊都會移除配對，之後不再自動連線。
- UI 的「停止連線」：停止目前 session，60 秒內不自動重連。
- UI 右上「結束程式」：停止 session 並結束常駐程式（服務仍在背景）。

移除程式
--------
1. 在 UI 按「結束程式」。
2. 以管理員執行：
   "C:\Program Files\LAN-KVM\lankvm-service.exe" uninstall
3. 刪除 C:\Program Files\LAN-KVM。
4. 刪除登錄檔 HKCU\Software\Microsoft\Windows\CurrentVersion\Run 的 LAN-KVM。
5. 刪除防火牆規則：LAN-KVM Discovery / Control / Data。
6. 刪除桌面與開始功能表的 LAN-KVM 捷徑。

目前限制
--------
- P0 傳輸是未加密 UDP；只可在信任的區域網路使用。
- 配對碼只有 3 位數，沒有防止暴力嘗試；正式版會換成 SPAKE2/憑證。
- 程式與服務未簽章，McAfee/Apex One 可能隔離；必要時請 IT 對檔案 hash 加白名單。
- 僅支援 Windows。

疑難排解
--------
- 掃不到對方：確認同網段、兩台都開著、AP 沒有 client isolation。
- 配對碼視窗沒出現：在 B 開啟 LAN-KVM 捷徑。
- 連線中斷：UI 按「收集紀錄」，看 app.log 與 engine.log。
- 登入畫面無法控制：確認 sc query LAN-KVM 是 RUNNING，且 service.log 有
  Winlogon: injector active。
- 舊 service 還占著 45871：管理員執行 sc stop LAN-KVM / sc delete LAN-KVM。
