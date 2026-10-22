$ErrorActionPreference = "Stop"
$root = $PSScriptRoot
$agent = Join-Path $root "lankvm-agent.exe"
$devtools = Join-Path $root "lankvm-devtools.exe"

Write-Host "=== probe ==="
& $agent @("probe")

Write-Host ""
Write-Host "=== absolute cursor ==="
Write-Host "Run this manually when the mouse is idle and the service is stopped:"
Write-Host "  .\lankvm-devtools.exe abs-test"

Write-Host ""
Write-Host "=== clipboard (text/image/file) ==="
$tmp = Join-Path $env:TEMP "lankvm-clip-test.txt"
Set-Content -Path $tmp -Value "LAN-KVM file clipboard test" -Encoding UTF8
$clip = Start-Process -FilePath $devtools `
    -ArgumentList @("clipboard-test", "--files", $tmp) `
    -NoNewWindow -Wait -PassThru
if ($clip.ExitCode -ne 0) {
    throw "clipboard-test failed with exit code $($clip.ExitCode)"
}

Write-Host ""
Write-Host "=== local file copy ==="
$src = Join-Path $env:TEMP "lankvm-file-test.bin"
$dst = Join-Path $env:TEMP "lankvm-file-out"
New-Item -ItemType Directory -Force $dst | Out-Null
$bytes = New-Object byte[] (3MB + 17)
for ($i = 0; $i -lt $bytes.Length; $i++) {
    $bytes[$i] = [byte](($i * 31) % 251)
}
[System.IO.File]::WriteAllBytes($src, $bytes)
$proc = Start-Process -FilePath $devtools `
    -ArgumentList @("file-test", "--source", $src, "--dest", $dst, "--overwrite") `
    -NoNewWindow -Wait -PassThru
if ($proc.ExitCode -ne 0) {
    throw "file-test failed with exit code $($proc.ExitCode)"
}

Write-Host ""
Write-Host "Local tests done."
Write-Host "Winlogon test is manual: install the service, run local-winlogon-test, then lock the screen."
