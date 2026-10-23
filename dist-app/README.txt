LAN-KVM 0.2（測試版）
=====================

需求
----
- Windows 10 / 11 x64
- 兩台電腦在同一個區域網路（同一個 AP / Switch）
- 兩台都安裝本程式

安裝
----
1. 解壓縮這個資料夾。
2. 執行 setup.exe（會跳 UAC，按「是」）。
3. 安裝到 C:\Program Files\LAN-KVM，並自動：
   - 建立防火牆規則：UDP 45870 / 45871 / 45872（不需要再手動加）
   - 設定開機自動啟動（--silent）
   - 建立桌面與開始功能表的 LAN-KVM 捷徑
   - 移除舊的 LAN-KVM 服務（避免佔用 45871）
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
- 門檻可調：
  - 切入推力 px（預設 20）
  - 時間窗 ms（預設 300）
  - 回切越界 px（預設 2，存在 B 的設定）

設定與資料
----------
- 配對、名稱、門檻：%LOCALAPPDATA%\LAN-KVM\app.json
- 程式日誌：%LOCALAPPDATA%\LAN-KVM\app.log
- 連線日誌：%LOCALAPPDATA%\LAN-KVM\engine.log

解除配對 / 停止
---------------
- UI 的「解除」：兩邊都會移除配對，之後不再自動連線。
- UI 的「停止連線」：停止目前 session，60 秒內不自動重連。
- UI 右上「結束程式」：停止 session 並結束常駐程式。

移除程式
--------
1. 在 UI 按「結束程式」。
2. 刪除 C:\Program Files\LAN-KVM 資料夾。
3. 刪除登錄檔 HKCU\Software\Microsoft\Windows\CurrentVersion\Run 的 LAN-KVM。
4. 刪除防火牆規則：LAN-KVM Discovery / Control / Data（可用 netsh 或防火牆進階設定）。
5. 刪除桌面與開始功能表的 LAN-KVM 捷徑。

目前限制
--------
- P0 傳輸是未加密 UDP；只可在信任的區域網路使用。
- 配對碼只有 3 位數，沒有防止暴力嘗試；正式版會換成 SPAKE2/憑證。
- 這一版只控制「已登入的桌面」；Windows 登入畫面 / UAC 的 SYSTEM 服務整合尚未接進這個 App。
- 程式未簽章，McAfee/Apex One 可能隔離；必要時請 IT 對檔案 hash 加白名單。
- 僅支援 Windows。

疑難排解
--------
- 掃不到對方：
  - 確認兩台同網段、都開著 LAN-KVM。
  - 確認 Windows 網路設定為「私人」或防火牆規則存在。
  - 關閉 AP 的 client isolation。
- 配對碼視窗沒出現：在 B 開啟 LAN-KVM 捷徑。
- 連線中斷：看 %LOCALAPPDATA%\LAN-KVM\app.log 與 engine.log。
- 舊 service 還占著 45871：以管理員執行 sc stop LAN-KVM / sc delete LAN-KVM。
