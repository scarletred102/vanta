[CmdletBinding()]
param(
    [switch]$Virtio,
    [switch]$Network,
    [ValidateRange(5, 60)]
    [int]$TimeoutSeconds = 30
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$qemu = if ($env:QEMU) { $env:QEMU } else { "qemu-system-x86_64" }
$ovmf = if ($env:OVMF) { $env:OVMF } else { "C:\Program Files\qemu\share\edk2-x86_64-code.fd" }
$esp = (Resolve-Path .\esp).Path
$log = Join-Path $env:TEMP "vanta-qemu-test.log"
$disk = Join-Path $env:TEMP "vanta-qemu-test-virtio.img"
$tcpProbe = $null

$env:BUILD_ONLY = "1"
try {
    & .\run.ps1
    if ($LASTEXITCODE -ne 0) {
        throw "kernel build failed with exit code $LASTEXITCODE"
    }
} finally {
    Remove-Item Env:BUILD_ONLY -ErrorAction SilentlyContinue
}

Remove-Item -LiteralPath $log -Force -ErrorAction SilentlyContinue
$arguments = @(
    "-drive", "if=pflash,format=raw,readonly=on,file=`"$ovmf`"",
    "-drive", "format=raw,file=fat:rw:`"$esp`",if=ide",
    "-serial", "file:$log",
    "-smp", "2",
    "-m", "256M",
    "-no-reboot", "-no-shutdown", "-display", "none"
)

if ($Virtio) {
    Remove-Item -LiteralPath $disk -Force -ErrorAction SilentlyContinue
    $stream = [System.IO.File]::Open(
        $disk,
        [System.IO.FileMode]::Create,
        [System.IO.FileAccess]::ReadWrite,
        [System.IO.FileShare]::None
    )
    try {
        $stream.SetLength(8MB)
    } finally {
        $stream.Dispose()
    }
    $arguments += @(
        "-drive", "file=$disk,if=none,format=raw,id=vd0",
        "-device", "virtio-blk-pci,disable-modern=on,ioeventfd=off,drive=vd0"
    )
}
if ($Network) {
    $arguments += @(
        "-netdev", "user,id=net0",
        "-device", "virtio-net-pci,disable-modern=on,ioeventfd=off,netdev=net0"
    )
}

function Invoke-QemuBoot {
    param([string[]]$CompletionMarkers)

    Remove-Item -LiteralPath $log -Force -ErrorAction SilentlyContinue
    $process = Start-Process -FilePath $qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    $output = ""
    try {
        $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
        while ((Get-Date) -lt $deadline) {
            if (Test-Path -LiteralPath $log) {
                $output = Get-Content -LiteralPath $log -Raw -ErrorAction SilentlyContinue
                if ($null -eq $output) {
                    $output = ""
                }
                $complete = $true
                foreach ($marker in $CompletionMarkers) {
                    if (!$output.Contains($marker)) {
                        $complete = $false
                        break
                    }
                }
                if ($complete) {
                    return $output
                }
            }
            if ($process.HasExited) {
                break
            }
            Start-Sleep -Milliseconds 100
        }
    } finally {
        if (!$process.HasExited) {
            Stop-Process -Id $process.Id -Force
        }
    }
    if (!(Test-Path -LiteralPath $log)) {
        throw "QEMU did not create a serial log"
    }
    if (!$output) {
        $output = Get-Content -LiteralPath $log -Raw
    }
    if ($null -eq $output) {
        $output = ""
    }
    $output
}

function Start-TcpProbe {
    param([int]$ExpectedConnections)

    $script:tcpProbe = Start-Job -ArgumentList $ExpectedConnections -ScriptBlock {
        param([int]$ExpectedConnections)

        $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 18080)
        $listener.Start()
        Write-Output "ready"
        try {
            for ($probe = 0; $probe -lt $ExpectedConnections; $probe++) {
                $accept = $listener.AcceptTcpClientAsync()
                if (!$accept.Wait(15000)) {
                    throw "guest did not open TCP probe connection $($probe + 1)"
                }
                $client = $accept.GetAwaiter().GetResult()
                try {
                    $stream = $client.GetStream()
                    $stream.ReadTimeout = 10000
                    $received = New-Object byte[] 4
                    $offset = 0
                    while ($offset -lt $received.Length) {
                        $count = $stream.Read($received, $offset, $received.Length - $offset)
                        if ($count -eq 0) {
                            throw "guest closed TCP probe before sending ping"
                        }
                        $offset += $count
                    }
                    if ([System.Text.Encoding]::ASCII.GetString($received) -ne "ping") {
                        throw "unexpected TCP probe payload"
                    }
                    $reply = [System.Text.Encoding]::ASCII.GetBytes("pong")
                    $stream.Write($reply, 0, $reply.Length)
                    $stream.Flush()
                    $stream.ReadTimeout = 5000
                    $discard = New-Object byte[] 16
                    while ($stream.Read($discard, 0, $discard.Length) -ne 0) {}
                } finally {
                    $client.Dispose()
                }
            }
            Write-Output "complete"
        } finally {
            $listener.Stop()
        }
    }

    $deadline = (Get-Date).AddSeconds(5)
    while ((Get-Date) -lt $deadline) {
        $output = @(Receive-Job -Job $script:tcpProbe -Keep -ErrorAction SilentlyContinue)
        if ($output -contains "ready") {
            return
        }
        if ($script:tcpProbe.State -ne "Running") {
            throw "TCP probe did not start`n$output"
        }
        Start-Sleep -Milliseconds 50
    }
    throw "TCP probe did not become ready"
}

function Complete-TcpProbe {
    try {
        if ($null -eq (Wait-Job -Job $script:tcpProbe -Timeout 5)) {
            throw "TCP probe did not observe a guest connection"
        }
        $output = @(Receive-Job -Job $script:tcpProbe -ErrorAction SilentlyContinue)
        if ($script:tcpProbe.State -ne "Completed" -or !($output -contains "complete")) {
            throw "TCP probe failed`n$output"
        }
    } finally {
        if ($null -ne $script:tcpProbe) {
            if ($script:tcpProbe.State -eq "Running") {
                Wait-Job -Job $script:tcpProbe -Timeout 5 | Out-Null
            }
            if ($script:tcpProbe.State -ne "Running") {
                Remove-Job -Job $script:tcpProbe -Force
            }
            $script:tcpProbe = $null
        }
    }
}

try {
    if ($Network) {
        $expectedTcpConnections = if ($Virtio) { 2 } else { 1 }
        Start-TcpProbe $expectedTcpConnections
    }

    $required = @(
        "[storage] writable VFS root mounted and lifecycle/remount self-check passed",
        "[smp] queued AP run queue=true dispatched=2",
        "[smp] AP cpu=1 run queue complete",
        "[shell] entering main loop"
    )
    if ($Virtio) {
        $required += "[storage] virtio-blk ready:"
    }
    if ($Network) {
        $required += "[net] udp dns reply"
        $required += "[net] VFS configuration loaded"
        $required += "[net] tcp user socket probe passed"
    }
    $output = Invoke-QemuBoot -CompletionMarkers $required

    foreach ($marker in $required) {
        if (!$output.Contains($marker)) {
            throw "QEMU regression failed: missing '$marker'`n$output"
        }
    }

    if ($Virtio) {
        $second_boot_required = @("[storage] persistent VFS mounted: existed=true")
        if ($Network) {
            $second_boot_required += "[net] VFS configuration loaded"
            $second_boot_required += "[net] tcp user socket probe passed"
        }
        $second_boot = Invoke-QemuBoot -CompletionMarkers $second_boot_required
        if (!$second_boot.Contains("[storage] persistent VFS mounted: existed=true")) {
            throw "QEMU persistence regression failed: disk was not remounted on the second boot`n$second_boot"
        }
        if ($Network) {
            foreach ($marker in @("[net] VFS configuration loaded", "[net] tcp user socket probe passed")) {
                if (!$second_boot.Contains($marker)) {
                    throw "QEMU second-boot network regression failed: missing '$marker'`n$second_boot"
                }
            }
        }
    }

    if ($Network) {
        Complete-TcpProbe
    }

    Write-Host "[test] QEMU regression passed (virtio=$Virtio network=$Network)"
} finally {
    if ($null -ne $tcpProbe) {
        if ($tcpProbe.State -eq "Running") {
            Wait-Job -Job $tcpProbe -Timeout 5 | Out-Null
        }
        if ($tcpProbe.State -ne "Running") {
            Remove-Job -Job $tcpProbe -Force
        }
    }
}
