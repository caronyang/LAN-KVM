@echo off
rem Build the Inno Setup installer.
rem Requires: Inno Setup 6.7+ (per-user install works, no admin needed to build).
rem
rem   cargo build --release -p lankvm-app -p lankvm-service
rem   installer\build.cmd
rem
rem Output: dist-app\LAN-KVM-Setup-1.1.9.exe

setlocal
if "%ISCC%"=="" set "ISCC=%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe"
if not exist "%ISCC%" set "ISCC=%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe"
if not exist "%ISCC%" set "ISCC=%ProgramFiles%\Inno Setup 6\ISCC.exe"
if not exist "%ISCC%" (
  echo [error] ISCC.exe not found. Install Inno Setup 6 or set ISCC=...
  exit /b 1
)

"%ISCC%" "%~dp0lankvm.iss" %*
