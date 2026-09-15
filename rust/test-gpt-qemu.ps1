[CmdletBinding()]
param(
    [ValidateRange(5, 7200)]
    [int]$TimeoutSeconds = 3600,
    [switch]$SummaryOnly
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

function Invoke-XtaskImage {
    param([switch]$Quiet)
    if ($Quiet) {
        $targetDir = Join-Path $PSScriptRoot "target"
        if (!(Test-Path $targetDir)) { New-Item -ItemType Directory -Path $targetDir | Out-Null }
        $outLog = Join-Path $targetDir "xtask_build.log"
        $errLog = Join-Path $targetDir "xtask_build_err.log"
        $proc = Start-Process -FilePath "cargo" -ArgumentList "xtask", "image" -NoNewWindow -Wait -PassThru -RedirectStandardOutput $outLog -RedirectStandardError $errLog
        if ($proc.ExitCode -ne 0) {
            Get-Content -LiteralPath $errLog | ForEach-Object { Write-Host $_ }
            throw "GPT image build failed with exit code $($proc.ExitCode)"
        }
    } else {
        cargo xtask image
        if ($LASTEXITCODE -ne 0) {
            throw "GPT image build failed with exit code $LASTEXITCODE"
        }
    }
}

Invoke-XtaskImage -Quiet:$SummaryOnly

$image = (Resolve-Path .\target\vanta-gpt.img).Path
$manifest = (Resolve-Path .\target\vanta-gpt.manifest).Path
$imageHash = (Get-FileHash -LiteralPath $image -Algorithm SHA256).Hash
$manifestHash = (Get-FileHash -LiteralPath $manifest -Algorithm SHA256).Hash

Invoke-XtaskImage -Quiet:$SummaryOnly
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
        "-netdev", "user,id=net0,hostfwd=tcp::8080-:8080",
        "-device", "virtio-net-pci,disable-modern=on,ioeventfd=off,netdev=net0",
        "-device", "virtio-rng-pci",
        "-serial", "file:$log",
        "-smp", "2",
        "-m", "256M",
        "-no-reboot", "-no-shutdown", "-display", "none"
    )
    $process = Start-Process -FilePath $qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    try {
        $output = ""
        $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
        $hostCurlAttempted = $false
        while ((Get-Date) -lt $deadline) {
            if (Test-Path -LiteralPath $log) {
                $output = Get-Content -LiteralPath $log -Raw -ErrorAction SilentlyContinue
                if ($null -eq $output) { $output = "" }
                if (!$hostCurlAttempted -and $output.Contains("waiting for host curl request on port 8080")) {
                    $hostCurlAttempted = $true
                    Start-Sleep -Milliseconds 500
                    for ($attempt = 1; $attempt -le 5; $attempt++) {
                        try {
                            $resp = & curl.exe -s --max-time 3 http://localhost:8080
                            if ($resp -match "Hello Vanta!") {
                                Write-Host "[test] host curl verified: '$resp'"
                                break
                            }
                        } catch {
                            Start-Sleep -Milliseconds 500
                        }
                    }
                }
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
    "[rng] hardware entropy source: virtio-rng",
    "[storage] RedoxFS root mounted",
    "[storage] RedoxFS persistence check: true",
    "[swap] watermark reached: evicting page 0x50000000 to slot 0",
    "[swap] page-in from disk: vaddr=0x50000000 slot=0",
    "[swap] memory pressure eviction and swap-in verified: slot=0 data-verified=true",
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
    "[linux-dynamic] futex/mutex TSC spin-barrier contention verified",
    "[linux-dynamic] futex synchronization passed",
    "[linux-dynamic] thread joined successfully",
    "[linux-dynamic] timer subsystem and nanosleep verified",
    "[linux-dynamic] 85-syscall matrix and signals verified",
    "[dynamic-shlib] SUCCESS: cross-boundary call to libcalc.so verified (84, 1337)",
    "[dynamic-shlib] SUCCESS: cross-boundary data relocation verified",
    "[linux-dynamic] thread spawned",
    "[net] virtio-net adapter initialized",
    "[net-test] sub-threshold test: 10/10 packets received, 0 drops",
    "[net-test] burst test: 500 packets sent, 500 received in exact sequential order (0 loss, 0 reordering)",
    "[net-test] NAPI coalescing: 0 drops under 1000 pps threshold, 0 drops under burst load (PASS)",
    "[net-test] CPU utilization: active-polling=",
    "[linux-dynamic] network acceptance passed",
    "[http-server] starting BSD socket lifecycle tests...",
    "[http-server] PASS: socket options (SO_REUSEADDR, TCP_NODELAY)",
    "[http-server] PASS: bind and getsockname (127.0.0.1:8080)",
    "[http-server] PASS: listen(backlog=128)",
    "[http-server] PASS: accept4 and getpeername",
    "[http-server] PASS: HTTP GET request-response payload verified",
    "[http-server] PASS: graceful FIN/EOF teardown",
    "[http-server] PASS: EPIPE delivered on closed socket write",
    "[http-server] PASS: bind without SO_REUSEADDR correctly failed with EADDRINUSE",
    "[http-server] PASS: TIME_WAIT port reuse with SO_REUSEADDR succeeded",
    "[http-server] PASS: 3 concurrent client connections verified (concurrent data in flight across 3 active sockets)",
    "[http-server] PASS: simultaneous close (CLOSING -> TIME_WAIT -> CLOSED)",
    "[http-server] PASS: SYN queue timeout (purged after 3000ms verified via /proc/net/tcp)",
    "[http-server] PASS: SYN cookie protection under saturated backlog verified",
    "[http-server] PASS: host curl request served successfully with 'Hello Vanta!'",
    "[http-server] ALL TESTS PASSED",
    "[dns-test] starting RFC 1035 DNS resolver test suite...",
    "[dns-test] PASS: static resolution (localhost -> 127.0.0.1)",
    "[dns-test] PASS: positive lookup (example.com -> ",
    "[dns-test] PASS: TTL cache hit (0 outbound packets on second lookup)",
    "[dns-test] PASS: gethostbyname(example.com) verified",
    "[dns-test] PASS: NXDOMAIN for nonexistent domain",
    "[dns-test] ALL TESTS PASSED",
    "[dhcp] state=REQUESTING: received synthetic DHCPNAK: restarting discovery (Init)",
    "[dhcp-test] starting RFC 2131 DHCP client verification suite...",
    "[dhcp-test] PASS: DHCP state BOUND verified",
    "[dhcp-test] PASS: dynamic IP lease (10.0.2.15) verified",
    "[dhcp-test] PASS: subnet mask and gateway (10.0.2.2) verified",
    "[dhcp-test] PASS: DNS server option (10.0.2.3) verified",
    "[dhcp-test] PASS: socket bind to dynamically leased IP verified",
    "[dhcp-test] PASS: synthetic DHCP NAK recovery verified (restarted in Init and bound)",
    "[dhcp-test] ALL TESTS PASSED",
    "[afunix-test] starting AF_UNIX test suite...",
    "[afunix-test] PASS: stream socketpair bidirectional and EOF",
    "[afunix-test] PASS: datagram socketpair message boundaries",
    "[afunix-test] PASS: named VFS path bind and connect",
    "[afunix-test] PASS: SCM_RIGHTS file descriptor passing",
    "[afunix-test] PASS: abstract unix socket bind and connect",
    "[afunix-test] PASS: abstract socket auto-release on close verified",
    "[afunix-test] PASS: abstract socket raw-byte sequence with embedded null",
    "[afunix-test] ALL TESTS PASSED",
    "[afunix-receiver] PASS: SCM_RIGHTS file descriptor passing across unrelated processes",
    "[afunix-sender] PASS: receiver confirmed verification of passed fd",
    "[linux] afunix-sender: spawn=",
    "[linux] afunix-receiver: spawn=",
    "[linux] wget: spawn=",
    "[wget] TLS handshake completed successfully!",
    "[wget] Certificate chain verified against /etc/ssl/certs",
    "[net] TLS 1.3 outbound HTTPS download verified: /test.txt saved to RedoxFS",
    "[linux-fork] 50-iteration fork loop verified",
    "[linux-fork] Vector 1: 1000-fork 10MB COW stress verified",
    "[linux-fork] COW fork and waitpid verified",
    "[fault] user task killed by SIGSEGV",
    "[linux-fork] unmapped read fault SIGSEGV verified",
    "[linux-fork] unmapped write fault SIGSEGV verified",
    "[linux-fork] read-only mapped page write SIGSEGV verified",
    "[linux-fork] concurrent COW race 2000-iteration test verified",
    "[linux-fork] stack auto-expansion verified",
    "[linux-fork] Vector 3: 8MB stack auto-expansion verified",
    "[linux-fork] anonymous demand paging verified",
    "[linux-fork] Vector 2: 128MB demand paging verified",
    "[linux-fork] Vector 4: interactive preemption vs 4 CPU thrashers at priority 16 verified",
    "[linux-fork] demand-paged process exit and address space destruction verified",
    "[proc] destroy_address_space space=",
    "[linux-epoll] epoll and eventfd multiplexing verified",
    "[linux-proc] /proc virtual filesystem verified",
    "[mount-test] starting Dynamic Multi-Mount Verification (Test Vector 1)...",
    "[mount-test] PASS: mounted secondary tmpfs at /mnt/ram",
    "[mount-test] PASS: wrote 10 MiB to /mnt/ram/test_10mb.bin",
    "[mount-test] PASS: memory dropped by at least 10 MiB verified",
    "[mount-test] PASS: 10 MiB data verified bit-for-bit",
    "[mount-test] PASS: umount2(/mnt/ram) succeeded",
    "[mount-test] PASS: memory completely reclaimed verified",
    "[mount-test] PASS: /mnt/ram unmounted and inaccessible",
    "[mount-test] ALL TESTS PASSED",
    "[symlink-test] starting Symbolic Links & Dentry Cache Verification (Phase 4 Unit 2)...",
    "[symlink-test] PASS: symlink(/tmp/sym_target.txt, /tmp/sym_link) succeeded",
    "[symlink-test] PASS: readlink() returned exact target string",
    "[symlink-test] PASS: lstat (S_IFLNK) vs stat (S_IFREG) verified",
    "[symlink-test] PASS: bit-for-bit read through symlink verified",
    "[symlink-test] PASS: cross-mount symlink read and write verified",
    "[symlink-test] PASS: reverse cross-mount (tmpfs -> root) verified",
    "[symlink-test] PASS: exact 40-hop symlink chain succeeded",
    "[symlink-test] PASS: exact 41-hop chain returned ELOOP (40)",
    "[symlink-test] PASS: circular symlink loop returned ELOOP",
    "[symlink-test] PASS: broken symlink returned ENOENT on open, S_ISLNK on lstat",
    "[symlink-test] PASS: 200 repeated lookups hit dentry cache successfully",
    "[symlink-test] PASS: dentry cache invalidation on unlink verified",
    "[symlink-test] ALL TESTS PASSED SUCCESSFULLY (Exit Code 0)",
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

$firstRequired = $common + @(
    "[storage] RedoxFS reboot persistence marker: false",
    "[cache-durability] PASS: zero-copy mmap(MAP_SHARED) coherence with read()/write() verified",
    "[cache-durability] PASS: cross-process MAP_SHARED memory and read() coherence verified",
    "[cache-durability] INFO: in-memory write throughput: ",
    "[cache-durability] PASS: fsync() completed",
    "[cache-durability] PASS: Phase 1 complete, ready for simulated power loss",
    "[flusher-test] PASS: sync() system call committed page cache to disk",
    "[flusher-test] PASS: background flusher daemon committed dirty pages after 500ms sleep",
    "[flusher-test] PASS: LRU clean page eviction under memory pressure verified",
    "[flusher-test] ALL TESTS PASSED SUCCESSFULLY (Exit Code 0)",
    "[proc-conformance] PASS: /proc/self/exe resolves to valid ELF binary",
    "[proc-conformance] PASS: /proc/self/maps displays valid VMA layout",
    "[proc-conformance] PASS: /dev/null discards writes and returns EOF on read",
    "[proc-conformance] PASS: /dev/zero returns continuous 0x00 bytes",
    "[proc-conformance] PASS: /dev/urandom furnishes non-deterministic CSPRNG entropy",
    "[proc-conformance] PASS: system telemetry streams furnish accurate runtime stats",
    "[proc-conformance] ALL TESTS PASSED SUCCESSFULLY (Exit Code 0)",
    "[pty-test] PASS: Master/slave pair opened successfully",
    "[pty-test] PASS: Canonical line discipline & backspace editing verified",
    "[pty-test] PASS: Raw mode pass-through verified",
    "[pty-test] PASS: TIOCSWINSZ updated winsize and injected SIGWINCH",
    "[pty-test] PASS: Ctrl+C injected SIGINT into foreground process group",
    "[pty-test] PASS: Master close produced EOF on slave read",
    "[pty-test] ALL TESTS PASSED SUCCESSFULLY (Exit Code 0)"
)
$first = Invoke-GptBoot -DiskImage $image -Label "first boot" -Required $firstRequired
Start-Sleep -Milliseconds 500
$secondRequired = $common + @(
    "[storage] RedoxFS reboot persistence marker: true",
    "[cache-durability] PASS: reboot persistence verified, 50 MiB sha256 bit-for-bit match",
    "[cache-durability] ALL TESTS PASSED SUCCESSFULLY (Exit Code 0)"
)
$second = Invoke-GptBoot -DiskImage $image -Label "reboot persistence" -Required $secondRequired

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

Write-Host "[test] GPT Gate A, Gate B, Gate C, Gate D, Gate E, Gate F, and Gate H acceptance passed"
if ($SummaryOnly) {
    Write-Host "[test] === SUMMARY MODE: All $($firstRequired.Count + $secondRequired.Count) assertions verified ==="
    $first -split "`n" | Where-Object { $_ -match "\[cache-durability\]|\[flusher-test\]" } | ForEach-Object { Write-Host $_ }
    $second -split "`n" | Where-Object { $_ -match "\[cache-durability\]" } | ForEach-Object { Write-Host $_ }
} else {
    $first -split "`n" | Where-Object { $_ -match "afunix|SIGSEGV|linux-fork|destroy_address_space|Vector|swap|dynamic-shlib|dynamic-threads|spin-barrier|rounds=|net-test|virtio-net|http-server|wget|mount-test|symlink-test|cache-durability|flusher-test|proc-conformance|pty-test" } | ForEach-Object { Write-Host $_ }
}
