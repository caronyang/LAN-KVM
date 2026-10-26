; LAN-KVM installer (Inno Setup 6.7+)
;
; Build:
;   cargo build --release -p lankvm-app -p lankvm-service
;   installer\build.cmd
;
; Output: dist-app\LAN-KVM-Setup-0.4-inno.exe
;
; The install steps deliberately mirror crates/lankvm-setup (the previous
; self-extracting setup) so an Inno install is a drop-in replacement:
;   * kill the running tray app / old service before replacing files
;   * lankvm-service.exe install   (create-or-update + start, AutoStart)
;   * firewall rules for UDP 45870-45872 + TCP 45873 / 45875
;   * ONSTART task + 5-minute ensure task for the service
;   * SCM recovery (restart on crash)
;   * HKCU Run entry for the tray app

#define AppName "LAN-KVM"
#define AppVersion "1.1.0"
#define AppPublisher "LAN-KVM"
#define SourceDir "..\target\release"

[Setup]
AppId={{8B0F6E1C-4A2B-4C57-9A3B-7E5D2C1F9A10}
AppName={#AppName}
AppVersion={#AppVersion}
VersionInfoVersion=1.1.0.0
VersionInfoDescription=LAN-KVM Setup
AppPublisher={#AppPublisher}
AppPublisherURL=https://github.com/caronyang/LAN-KVM
AppSupportURL=https://github.com/caronyang/LAN-KVM
DefaultDirName={autopf}\LAN-KVM
DefaultGroupName=LAN-KVM
DisableProgramGroupPage=yes
OutputDir=..\dist-app
OutputBaseFilename=LAN-KVM-Setup-1.1.0
SetupIconFile=..\assets\branding\icons\lankvm.ico
UninstallDisplayIcon={app}\lankvm-app.exe
UninstallDisplayName={#AppName} {#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
CloseApplications=no
RestartApplications=no
SetupLogging=yes

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "zh"; MessagesFile: "ChineseTraditional.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#SourceDir}\lankvm-app.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\lankvm-service.exe"; DestDir: "{app}"; Flags: ignoreversion
; The SYSTEM service needs client_port to run the P0 client engine.
Source: "config.txt"; DestDir: "{commonappdata}\LAN-KVM"; Flags: onlyifdoesntexist uninsneveruninstall

[Icons]
Name: "{autoprograms}\LAN-KVM"; Filename: "{app}\lankvm-app.exe"
Name: "{autoprograms}\{cm:UninstallProgram,LAN-KVM}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\LAN-KVM"; Filename: "{app}\lankvm-app.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; \
    ValueName: "LAN-KVM"; ValueData: """{app}\lankvm-app.exe"" --silent"; Flags: uninsdeletevalue
Root: HKLM; Subkey: "Software\LAN-KVM"; ValueType: string; ValueName: "InstallDir"; \
    ValueData: "{app}"; Flags: uninsdeletekey

[Run]
Filename: "{app}\lankvm-app.exe"; Parameters: "--silent"; \
    Description: "{cm:LaunchProgram,LAN-KVM}"; \
    Flags: nowait postinstall skipifsilent runasoriginaluser
; Silent installs (remote deployment) also start the tray app for the current user.
Filename: "{app}\lankvm-app.exe"; Parameters: "--silent"; \
    Flags: nowait runasoriginaluser; Check: WizardSilent

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM lankvm-app.exe /T"; Flags: runhidden; RunOnceId: "KillApp"
Filename: "{app}\lankvm-service.exe"; Parameters: "uninstall"; Flags: runhidden waituntilterminated; RunOnceId: "RemoveService"
Filename: "{sys}\schtasks.exe"; Parameters: "/Delete /TN ""LAN-KVM Boot Start"" /F"; Flags: runhidden; RunOnceId: "DelBootTask"
Filename: "{sys}\schtasks.exe"; Parameters: "/Delete /TN ""LAN-KVM Ensure Service"" /F"; Flags: runhidden; RunOnceId: "DelEnsureTask"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""LAN-KVM Discovery"""; Flags: runhidden; RunOnceId: "DelFwDiscovery"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""LAN-KVM Control"""; Flags: runhidden; RunOnceId: "DelFwControl"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""LAN-KVM Data"""; Flags: runhidden; RunOnceId: "DelFwData"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""LAN-KVM Logs"""; Flags: runhidden; RunOnceId: "DelFwLogs"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""LAN-KVM Clipboard"""; Flags: runhidden; RunOnceId: "DelFwClipboard"

[Code]
var
  ScPath: String;

procedure SetupLog(const Msg: String);
begin
  Log(Msg);
  SaveStringToFile(GetEnv('TEMP') + '\lankvm-setup.log',
    '[' + GetDateTimeString('yyyy-mm-dd hh:nn:ss', '-', ':') + '] ' + Msg + #13#10,
    True);
end;

function RunTool(const FileName, Params: String): Boolean;
var
  Code: Integer;
begin
  Result := Exec(FileName, Params, '', SW_HIDE, ewWaitUntilTerminated, Code);
  if Result then
    Result := (Code = 0);
  SetupLog('run ' + ExtractFileName(FileName) + ' ' + Params +
           ' -> ok=' + IntToStr(Ord(Result)) + ' exit=' + IntToStr(Code));
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  SetupLog('inno: install start');
  ScPath := ExpandConstant('{sys}\sc.exe');

  // 1. Stop the tray app and every engine child process.
  RunTool(ExpandConstant('{sys}\taskkill.exe'), '/F /IM lankvm-app.exe /T');
  Sleep(400);

  // 2. Stop the service synchronously; `net stop` waits until SCM reports
  //    STOPPED, so the service binary is no longer locked.
  RunTool(ExpandConstant('{sys}\net.exe'), 'stop LAN-KVM');
  Sleep(800);

  // 3. Kill leftover session agents / a wedged service process.
  RunTool(ExpandConstant('{sys}\taskkill.exe'), '/F /IM lankvm-service.exe /T');
  Sleep(600);
end;

procedure CreateServiceTask(const TaskName, Schedule, Extra: String);
var
  Cmd, Params: String;
begin
  Cmd := '"' + ScPath + '" start LAN-KVM';
  Params := '/Create /TN "' + TaskName + '" /SC ' + Schedule + ' ' + Extra +
            ' /RU SYSTEM /RL HIGHEST /TR "' + Cmd + '" /F';
  RunTool(ExpandConstant('{sys}\schtasks.exe'), Params);
end;

procedure AddFirewallRule(const Name, Protocol, Port: String);
begin
  RunTool(ExpandConstant('{sys}\netsh.exe'),
    'advfirewall firewall delete rule name="' + Name + '"');
  RunTool(ExpandConstant('{sys}\netsh.exe'),
    'advfirewall firewall add rule name="' + Name +
    '" dir=in action=allow protocol=' + Protocol + ' localport=' + Port);
end;

procedure ConfigureSystem;
var
  ServiceExe: String;
begin
  ServiceExe := ExpandConstant('{app}\lankvm-service.exe');

  // Service: create-or-update, AutoStart, start. Logs go to
  // C:\ProgramData\LAN-KVM\logs.
  if RunTool(ServiceExe, 'install') then
    SetupLog('inno: service install -> true')
  else
    SetupLog('inno: service install -> false');

  // Inbound: discovery / control / input / log upload / clipboard.
  AddFirewallRule('LAN-KVM Discovery', 'UDP', '45870');
  AddFirewallRule('LAN-KVM Control', 'UDP', '45871');
  AddFirewallRule('LAN-KVM Data', 'UDP', '45872');
  AddFirewallRule('LAN-KVM Logs', 'TCP', '45873');
  AddFirewallRule('LAN-KVM Clipboard', 'TCP', '45875');

  // Boot safety net: SCM AutoStart plus an ONSTART task and a 5-minute
  // ensure task (same as the previous setup).
  CreateServiceTask('LAN-KVM Boot Start', 'ONSTART', '');
  CreateServiceTask('LAN-KVM Ensure Service', 'MINUTE', '/MO 5');

  // Restart the service if it crashes.
  RunTool(ScPath, 'failure LAN-KVM reset= 86400 ' +
                  'actions= restart/5000/restart/15000/restart/60000');

  SetupLog('inno: install done');
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    ConfigureSystem;
end;
