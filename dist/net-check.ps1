param(
    [ValidateSet("info", "listen", "send")]
    [string]$Mode = "info",

    [string]$Peer = "",

    [int]$Port = 45871,

    [int]$Count = 10,

    [int]$Seconds = 60,

    [int]$TimeoutMs = 1000,

    [switch]$AddFirewallRule
)

$ErrorActionPreference = "Stop"

function Test-Admin {
    $p = New-Object Security.Principal.WindowsPrincipal(
        [Security.Principal.WindowsIdentity]::GetCurrent()
    )
    return $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Show-Info {
    Write-Host "=== LAN-KVM network check ===" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Host: $env:COMPUTERNAME"
    Write-Host ""

    Write-Host "--- IPv4 addresses ---"
    Get-NetIPAddress -AddressFamily IPv4 |
        Where-Object { $_.IPAddress -notlike "127.*" -and $_.IPAddress -notlike "169.254.*" } |
        Select-Object IPAddress, InterfaceAlias, PrefixLength |
        Format-Table -AutoSize | Out-String | Write-Host

    Write-Host "--- Firewall profiles ---"
    Get-NetFirewallProfile |
        Select-Object Name, Enabled |
        Format-Table -AutoSize | Out-String | Write-Host

    Write-Host "--- Firewall rules for UDP $Port ---"
    $rules = Get-NetFirewallRule -ErrorAction SilentlyContinue |
        Where-Object { $_.DisplayName -like "*LAN-KVM*" -or $_.DisplayName -like "*$Port*" }
    if ($rules) {
        $rules | Select-Object DisplayName, Enabled, Direction, Action |
            Format-Table -AutoSize | Out-String | Write-Host
    } else {
        Write-Host "  (none)"
    }

    Write-Host "--- UDP $Port listener ---"
    $ep = Get-NetUDPEndpoint -LocalPort $Port -ErrorAction SilentlyContinue
    if ($ep) {
        $ep | Select-Object LocalAddress, LocalPort, OwningProcess |
            Format-Table -AutoSize | Out-String | Write-Host
        foreach ($e in $ep) {
            $proc = Get-Process -Id $e.OwningProcess -ErrorAction SilentlyContinue
            if ($proc) { Write-Host ("  pid {0} = {1}" -f $proc.Id, $proc.ProcessName) }
        }
    } else {
        Write-Host "  (nobody is listening)"
    }

    Write-Host "--- Security / EDR services ---"
    Get-Service -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -match "Apex|Tmcc|SED|CSFalcon|carbon|McAPExe|McAfee" } |
        Select-Object Name, Status, DisplayName |
        Format-Table -AutoSize | Out-String | Write-Host

    Write-Host "Next:"
    Write-Host "  Listener (client machine, Administrator):"
    Write-Host "    .\run-net-check.cmd -Mode listen -Port $Port -AddFirewallRule"
    Write-Host "  Sender (master machine):"
    Write-Host "    .\run-net-check.cmd -Mode send -Peer <client-ip> -Port $Port"
    Write-Host ""
    Write-Host "If .ps1 is blocked by execution policy, use run-net-check.cmd"
    Write-Host "or run: Set-ExecutionPolicy -Scope Process Bypass -Force"
}

function Add-Rule {
    $name = "LAN-KVM P0 UDP $Port"
    $existing = Get-NetFirewallRule -DisplayName $name -ErrorAction SilentlyContinue
    if ($existing) {
        Write-Host "Firewall rule already exists: $name" -ForegroundColor Yellow
        return
    }
    New-NetFirewallRule -DisplayName $name -Direction Inbound -Protocol UDP `
        -LocalPort $Port -Action Allow -Profile Any | Out-Null
    Write-Host "Added inbound UDP $Port allow rule (all profiles)." -ForegroundColor Green
}

function Invoke-Listen {
    if ($AddFirewallRule) {
        if (-not (Test-Admin)) { throw "-AddFirewallRule needs an Administrator PowerShell." }
        Add-Rule
    }

    $udp = New-Object System.Net.Sockets.UdpClient($Port)
    $udp.Client.ReceiveTimeout = $TimeoutMs
    Write-Host "Listening on 0.0.0.0:$Port for $Seconds s ..." -ForegroundColor Cyan
    Write-Host "Press Ctrl+C to stop early."
    Write-Host ""

    $deadline = (Get-Date).AddSeconds($Seconds)
    $seen = 0
    while ((Get-Date) -lt $deadline) {
        try {
            $remote = New-Object System.Net.IPEndPoint([System.Net.IPAddress]::Any, 0)
            $bytes = $udp.Receive([ref]$remote)
        } catch [System.Net.Sockets.SocketException] {
            continue
        } catch {
            break
        }

        $seen++
        $ascii = -join ($bytes | ForEach-Object {
            if ($_ -ge 32 -and $_ -lt 127) { [char]$_ } else { "." }
        })
        Write-Host ("[{0}] {1} bytes from {2}: {3}" -f `
            (Get-Date -Format "HH:mm:ss"), $bytes.Length, $remote, $ascii)

        # Echo the payload back so the sender can measure RTT.
        [void]$udp.Send($bytes, $bytes.Length, $remote)
    }

    $udp.Close()
    Write-Host ""
    Write-Host "Stopped. Received $seen packet(s)."
    if ($seen -eq 0) {
        Write-Host "No packets arrived. Check subnet, firewall, EDR, and that the sender used this IP." -ForegroundColor Yellow
    }
}

function Invoke-Send {
    if ([string]::IsNullOrWhiteSpace($Peer)) {
        throw "-Mode send requires -Peer <client-ip>"
    }

    $target = New-Object System.Net.IPEndPoint(
        [System.Net.IPAddress]::Parse($Peer), $Port
    )
    $udp = New-Object System.Net.Sockets.UdpClient
    $udp.Client.ReceiveTimeout = $TimeoutMs
    $udp.Connect($target)

    Write-Host "Sending $Count LKVM Hello packets to $target ..." -ForegroundColor Cyan
    Write-Host ""

    $rtts = New-Object System.Collections.Generic.List[double]
    for ($i = 1; $i -le $Count; $i++) {
        $pkt = New-Object byte[] 28
        $pkt[0] = 0x4C; $pkt[1] = 0x4B; $pkt[2] = 0x56; $pkt[3] = 0x4D  # "LKVM"
        $pkt[4] = 1      # version
        $pkt[5] = 1      # kind = Hello
        $seq = [BitConverter]::GetBytes([uint32]$i)
        [Array]::Copy($seq, 0, $pkt, 8, 4)

        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        [void]$udp.Send($pkt, $pkt.Length)
        try {
            $remote = New-Object System.Net.IPEndPoint([System.Net.IPAddress]::Any, 0)
            [void]$udp.Receive([ref]$remote)
            $sw.Stop()
            $ms = $sw.Elapsed.TotalMilliseconds
            $rtts.Add($ms)
            Write-Host ("  seq {0,2}: reply in {1,6:N1} ms" -f $i, $ms) -ForegroundColor Green
        } catch {
            Write-Host ("  seq {0,2}: timeout after {1} ms" -f $i, $TimeoutMs) -ForegroundColor Red
        }
        Start-Sleep -Milliseconds 100
    }

    $udp.Close()
    Write-Host ""
    if ($rtts.Count -eq 0) {
        Write-Host "RESULT: FAIL - no replies." -ForegroundColor Red
        Write-Host "Check: same subnet? listener running? firewall UDP $Port inbound? EDR/AP isolation?"
        exit 1
    }

    $stats = $rtts | Measure-Object -Minimum -Maximum -Average
    Write-Host ("RESULT: PASS - {0}/{1} replies, min {2:N1} / avg {3:N1} / max {4:N1} ms" -f `
        $rtts.Count, $Count, $stats.Minimum, $stats.Average, $stats.Maximum) -ForegroundColor Green
    if ($rtts.Count -lt $Count) {
        Write-Host "Packet loss detected; note it before running the input PoC." -ForegroundColor Yellow
    }
}

switch ($Mode) {
    "info"   { Show-Info }
    "listen" { Invoke-Listen }
    "send"   { Invoke-Send }
}
