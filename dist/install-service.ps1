param(
    [int]$ClientPort = 0,
    [int]$ReleasePx = 2
)

$ErrorActionPreference = "Stop"

$principal = New-Object Security.Principal.WindowsPrincipal(
    [Security.Principal.WindowsIdentity]::GetCurrent()
)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Run this script from an Administrator PowerShell."
}

$exe = Join-Path $PSScriptRoot "lankvm-service.exe"
if (-not (Test-Path $exe)) {
    throw "Missing $exe"
}

if ($ClientPort -gt 0) {
    $dir = "C:\ProgramData\LAN-KVM"
    New-Item -ItemType Directory -Force $dir | Out-Null
    $lines = @("client_port=$ClientPort", "release_px=$ReleasePx")
    Set-Content (Join-Path $dir "config.txt") $lines
    Write-Host "Wrote client_port=$ClientPort release_px=$ReleasePx to $dir\config.txt"
}

& $exe install
sc.exe query LAN-KVM
Write-Host ""
Write-Host "Logs: C:\ProgramData\LAN-KVM\logs"
