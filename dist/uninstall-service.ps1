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

& $exe stop
& $exe uninstall
