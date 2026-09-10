[CmdletBinding()]
param(
    [ValidateRange(5, 180)]
    [int]$TimeoutSeconds = 90
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$qemu = if ($env:QEMU) { $env:QEMU } else { "qemu-system-x86_64" }
$ovmf = if ($env:OVMF -and (Test-Path $env:OVMF)) {
    $env:OVMF
} else {
    $candidates = @(
        "C:\msys64\ucrt64\share\qemu\edk2-x86_64-code.fd",
        "C:\Program Files\qemu\share\edk2-x86_64-code.fd",
        "C:\msys64\mingw64\share\qemu\edk2-x86_64-code.fd"
    )
    $found = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
    if ($found) { $found } else { "C:\Program Files\qemu\share\edk2-x86_64-code.fd" }
}
$log = Join-Path $env:TEMP "vanta-gpt-qemu-test.log"

if (!(Get-Command zig -ErrorAction SilentlyContinue)) {
    $zigCandidates = @(
        "C:\Users\rocki\AppData\Local\Microsoft\WinGet\Packages\zig.zig_Microsoft.Winget.Source_8wekyb3d8bbwe\zig-x86_64-windows-0.16.0",
        (Get-ChildItem -Path "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Filter "zig.exe" -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty DirectoryName)
    )
    $zigDir = $zigCandidates | Where-Object { $_ -and (Test-Path (Join-Path $_ "zig.exe")) } | Select-Object -First 1
    if ($zigDir) {
        $env:PATH = "$zigDir;$env:PATH"
    }
}

cargo xtask image
if ($LASTEXITCODE -ne 0) {
    throw "GPT image build failed with exit code $LASTEXITCODE"
}

$image = (Resolve-Path .\target\vanta-gpt.img).Path
$manifest = (Resolve-Path .\target\vanta-gpt.manifest).Path
$imageHash = (Get-FileHash -LiteralPath $image -Algorithm SHA256).Hash
$manifestHash = (Get-FileHash -LiteralPath $manifest -Algorithm SHA256).Hash
cargo xtask image
if ($LASTEXITCODE -ne 0) {
    throw "GPT reproducibility rebuild failed with exit code $LASTEXITCODE"
}
if ((Get-FileHash -LiteralPath $image -Algorithm SHA256).Hash -ne $imageHash -or
    (Get-FileHash -LiteralPath $manifest -Algorithm SHA256).Hash -ne $manifestHash) {
    throw "GPT image reproducibility mismatch"
}
Write-Host "[test] GPT image reproducibility passed"

function Invoke-GptBoot {
    param(
        [Parameter(Mandatory)] [string]$DiskImage,
        [Parameter(Mandatory)] [string[]]$Required,
        [Parameter(Mandatory)] [string]$Label
    )

    Remove-Item -LiteralPath $log -Force -ErrorAction SilentlyContinue
    $arguments = @(
        "-drive", "if=pflash,format=raw,readonly=on,file=`"$ovmf`"",
        "-drive", "file=`"$DiskImage`",if=none,format=raw,cache=writethrough,id=vd0",
        "-device", "virtio-blk-pci,disable-modern=on,ioeventfd=off,drive=vd0",
        "-netdev", "user,id=net0",
        "-device", "virtio-net-pci,disable-modern=on,ioeventfd=off,netdev=net0",
        "-serial", "file:$log",
        "-smp", "2",
        "-m", "256M",
        "-no-reboot", "-no-shutdown", "-display", "none"
    )
    $process = Start-Process -FilePath $qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    try {
        $output = ""
        $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
        while ((Get-Date) -lt $deadline) {
            if (Test-Path -LiteralPath $log) {
                $output = Get-Content -LiteralPath $log -Raw -ErrorAction SilentlyContinue
                if ($null -eq $output) { $output = "" }
                if (($Required | Where-Object { !$output.Contains($_) }).Count -eq 0) {
                    Write-Host "[test] GPT $Label passed"
                    return $output
                }
            }
            if ($process.HasExited) { break }
            Start-Sleep -Milliseconds 100
        }
        throw "GPT $Label failed. Serial log:`n$output"
    } finally {
        if (!$process.HasExited) {
            Stop-Process -Id $process.Id -Force
            try { $process.WaitForExit(3000) } catch {}
        }
    }
}

$common = @(
    "[storage] RedoxFS root mounted",
    "[storage] RedoxFS persistence check: true",
    "[proc] launching native /sbin/init",
    "[native] acceptance: developer-gate ok",
    "[native] terminal/filesystem acceptance passed",
    "vanta native shell",
    "hello from C on Vanta",
    "libvanta SDK smoke passed",
    "libvanta stdio smoke passed",
    "libvanta directory smoke passed",
    "libvanta environment smoke passed",
    "libvanta process smoke passed",
    "[native] acceptance: c-exec-smoke ok",
    "[procd] service registered",
    "[procd] service upgraded",
    "[procd] service discovered",
    "[procd] stale service authority revoked",
    "[procd] vfs backend passed",
    "[procd] service authority revoked",
    "[native] acceptance: procd-gate ok",
    "[native] acceptance: audit-persistence ok",
    "[native] Gate B IPC acceptance passed",
    "[linux] hello",
    "[linux-musl] hello",
    "[linux-musl] memory allocation passed",
    "[linux-musl] file io passed",
    "[linux-musl] directory iteration passed",
    "[linux-musl] pipes and descriptors passed",
    "[linux-musl] pipe throughput 64KB passed",
    "[linux-musl] process groups and termios ioctls verified",
    "[linux-musl] bad pointer negative tests verified",
    "[linux-musl] posix system info passed",
    "[linux-musl] script sequencing passed",
    "[linux-musl] socket execution passed",
    "[linuxd] unsupported syscall number=9999",
    "[linux] dynamic interpreter rejected",
    "[linux] Gate C personality acceptance passed",
    "[linux-dynamic] dynamic interpreter loaded",
    "[linux-dynamic] hello from dynamic musl/glibc",
    "[linux-dynamic] signal handler registered",
    "[linux-dynamic] signal delivered and handled",
    "[linux-dynamic] rt_sigreturn restored context",
    "[linux-dynamic] thread TLS verified",
    "[linux-dynamic] thread spawned",
    "[net] virtio-net adapter initialized",
    "[linux-dynamic] network acceptance passed",
    "[linux-fork] 50-iteration fork loop verified",
    "[linux-fork] COW fork and waitpid verified",
    "[fault] user task killed by SIGSEGV",
    "[linux-fork] invalid memory access SIGSEGV termination verified",
    "[linux-fork] concurrent COW race 50-iteration test verified",
    "[linux-fork] stack auto-expansion verified",
    "[linux-fork] anonymous demand paging verified",
    "[linux-fork] demand-paged process exit and address space destruction verified",
    "[proc] destroy_address_space space=",
    "[linux-epoll] epoll and eventfd multiplexing verified",
    "[linux-proc] /proc virtual filesystem verified",
    "desktop: GUI window surface composition verified",
    "audiod: PCM audio stream playback verified",
    "[orbital] window compositor initialized",
    "[orbital] z-order window management and drag-and-drop verified",
    "[orbterm] terminal emulator initialized on /bin/sh",
    "[orbital] desktop acceptance passed",
    "[busybox-sh] shell execution verified",
    "[linux-busybox] busybox suite verified",
    "[lua-runtime] hello from lua 5.4 scripting engine",
    "[vpkg] package manager v1.0 initialized",
    "[vpkg] package install and verification passed",
    "[linux] Gate D dynamic & networking acceptance passed"
)

$first = Invoke-GptBoot -DiskImage $image -Label "first boot" -Required ($common + "[storage] RedoxFS reboot persistence marker: false")
Start-Sleep -Milliseconds 500
$second = Invoke-GptBoot -DiskImage $image -Label "reboot persistence" -Required ($common + "[storage] RedoxFS reboot persistence marker: true")

$corruptRoot = Join-Path $env:TEMP "vanta-gpt-corrupt-root.img"
Copy-Item -LiteralPath $image -Destination $corruptRoot -Force
$corruptLength = (Get-Item -LiteralPath $corruptRoot).Length - (2 * 1024 * 1024)
$stream = [IO.File]::Open($corruptRoot, [IO.FileMode]::Open, [IO.FileAccess]::Write, [IO.FileShare]::Read)
try {
    $stream.SetLength($corruptLength)
} finally {
    $stream.Dispose()
}
Invoke-GptBoot -DiskImage $corruptRoot -Label "corrupt-root recovery" -Required @(
    "[recovery] entering kernel recovery shell",
    "[shell] entering main loop"
) | Out-Null
Remove-Item -LiteralPath $corruptRoot -Force -ErrorAction SilentlyContinue

Write-Host "[test] GPT Gate A, Gate B, Gate C, Gate D, and Gate E acceptance passed"
$first -split "`n" | Where-Object { $_ -match "SIGSEGV|linux-fork|destroy_address_space" } | ForEach-Object { Write-Host $_ }
