#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <sys/sysinfo.h>
#include <fcntl.h>
#include <errno.h>

const char __interp[] __attribute__((section(".interp"))) = "/lib/ld-musl-x86_64.so.1";

#ifndef SYS_mount
#define SYS_mount 165
#endif
#ifndef SYS_umount2
#define SYS_umount2 166
#endif

static void pmsg(const char *msg) {
    write(1, msg, strlen(msg));
}

int main(int argc, char **argv) {
    pmsg("[mount-test] starting Dynamic Multi-Mount Verification (Test Vector 1)...\n");

    struct sysinfo si_init;
    if (sysinfo(&si_init) != 0) {
        pmsg("[mount-test] FAIL: initial sysinfo failed\n");
        return 1;
    }

    // 1. Mount secondary tmpfs at /mnt/ram
    int rc = syscall(SYS_mount, "none", "/mnt/ram", "tmpfs", 0, NULL);
    if (rc != 0) {
        pmsg("[mount-test] FAIL: mount(/mnt/ram, tmpfs) failed\n");
        return 2;
    }
    pmsg("[mount-test] PASS: mounted secondary tmpfs at /mnt/ram\n");

    // 2. Query sysinfo after mount
    struct sysinfo si_mounted;
    if (sysinfo(&si_mounted) != 0) {
        pmsg("[mount-test] FAIL: sysinfo after mount failed\n");
        return 3;
    }

    // 3. Write 10 MiB to /mnt/ram/test_10mb.bin in 4 KiB chunks
    int fd = open("/mnt/ram/test_10mb.bin", O_CREAT | O_WRONLY | O_TRUNC, 0644);
    if (fd < 0) {
        pmsg("[mount-test] FAIL: open /mnt/ram/test_10mb.bin failed\n");
        return 4;
    }

    char chunk[4096];
    for (int i = 0; i < 4096; i++) {
        chunk[i] = (char)((i * 37 + 13) & 0xff);
    }

    const int total_chunks = 2560; // 2560 * 4096 = 10,485,760 bytes = 10 MiB
    for (int i = 0; i < total_chunks; i++) {
        chunk[0] = (char)(i & 0xff);
        chunk[1] = (char)((i >> 8) & 0xff);
        ssize_t written = write(fd, chunk, 4096);
        if (written != 4096) {
            pmsg("[mount-test] FAIL: write 10MiB chunk failed\n");
            close(fd);
            return 5;
        }
    }
    close(fd);
    pmsg("[mount-test] PASS: wrote 10 MiB to /mnt/ram/test_10mb.bin\n");

    // 4. Assert memory dropped by at least 10 MiB
    struct sysinfo si_written;
    if (sysinfo(&si_written) != 0) {
        pmsg("[mount-test] FAIL: sysinfo after write failed\n");
        return 6;
    }

    unsigned long long diff_bytes = 0;
    if (si_mounted.freeram >= si_written.freeram) {
        diff_bytes = (unsigned long long)si_mounted.freeram - (unsigned long long)si_written.freeram;
    }
    if (diff_bytes < 10000000ULL) {
        pmsg("[mount-test] FAIL: freeram did not drop by 10 MiB\n");
        return 7;
    }
    pmsg("[mount-test] PASS: memory dropped by at least 10 MiB verified\n");

    // 5. Read back and verify bit-for-bit
    fd = open("/mnt/ram/test_10mb.bin", O_RDONLY);
    if (fd < 0) {
        pmsg("[mount-test] FAIL: reopen /mnt/ram/test_10mb.bin for read failed\n");
        return 8;
    }
    char read_buf[4096];
    for (int i = 0; i < total_chunks; i++) {
        chunk[0] = (char)(i & 0xff);
        chunk[1] = (char)((i >> 8) & 0xff);
        ssize_t rd = read(fd, read_buf, 4096);
        if (rd != 4096 || memcmp(read_buf, chunk, 4096) != 0) {
            pmsg("[mount-test] FAIL: data verification mismatch\n");
            close(fd);
            return 9;
        }
    }
    close(fd);
    pmsg("[mount-test] PASS: 10 MiB data verified bit-for-bit\n");

    // 6. Unmount via umount2
    rc = syscall(SYS_umount2, "/mnt/ram", 0);
    if (rc != 0) {
        pmsg("[mount-test] FAIL: umount2(/mnt/ram) failed\n");
        return 10;
    }
    pmsg("[mount-test] PASS: umount2(/mnt/ram) succeeded\n");

    // 7. Assert memory is completely reclaimed
    struct sysinfo si_after;
    if (sysinfo(&si_after) != 0) {
        pmsg("[mount-test] FAIL: sysinfo after umount failed\n");
        return 11;
    }
    long long reclaim_diff = (long long)si_mounted.freeram - (long long)si_after.freeram;
    if (reclaim_diff < 0) reclaim_diff = -reclaim_diff;
    if (reclaim_diff > 65536) {
        pmsg("[mount-test] FAIL: memory was not completely reclaimed after umount\n");
        return 12;
    }
    pmsg("[mount-test] PASS: memory completely reclaimed verified\n");

    // 8. Assert file is gone
    if (access("/mnt/ram/test_10mb.bin", F_OK) == 0) {
        pmsg("[mount-test] FAIL: /mnt/ram/test_10mb.bin still accessible after umount\n");
        return 13;
    }
    pmsg("[mount-test] PASS: /mnt/ram unmounted and inaccessible\n");

    pmsg("[mount-test] ALL TESTS PASSED\n");
    return 0;
}
