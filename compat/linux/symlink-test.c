#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <sys/stat.h>
#include <sys/syscall.h>

#ifndef SYS_mount
#define SYS_mount 165
#endif
#ifndef SYS_umount2
#define SYS_umount2 166
#endif

static void pmsg(const char *msg) {
    write(1, msg, strlen(msg));
}

static void die(const char *msg, int code) {
    pmsg(msg);
    pmsg("\n");
    exit(code);
}

int main(int argc, char **argv) {
    (void)argc;
    (void)argv;
    pmsg("[symlink-test] starting Symbolic Links & Dentry Cache Verification (Phase 4 Unit 2)...\n");

    // =========================================================================
    // 1. Basic symlink creation, readlink, lstat vs stat, read/write via symlink
    // =========================================================================
    pmsg("[symlink-test] Test 1: Basic symlink creation & resolution...\n");
    unlink("/tmp/sym_target.txt");
    unlink("/tmp/sym_link");

    int fd = open("/tmp/sym_target.txt", O_CREAT | O_WRONLY | O_TRUNC, 0644);
    if (fd < 0) {
        die("[symlink-test] FAIL: failed to create /tmp/sym_target.txt", 1);
    }
    const char *orig_data = "Hello from symlink target file!";
    if (write(fd, orig_data, strlen(orig_data)) != (ssize_t)strlen(orig_data)) {
        close(fd);
        die("[symlink-test] FAIL: failed to write to /tmp/sym_target.txt", 2);
    }
    close(fd);

    if (symlink("/tmp/sym_target.txt", "/tmp/sym_link") != 0) {
        die("[symlink-test] FAIL: symlink() returned non-zero", 3);
    }
    pmsg("[symlink-test] PASS: symlink(/tmp/sym_target.txt, /tmp/sym_link) succeeded\n");

    char rl_buf[128];
    memset(rl_buf, 0, sizeof(rl_buf));
    ssize_t rl_len = readlink("/tmp/sym_link", rl_buf, sizeof(rl_buf) - 1);
    if (rl_len != (ssize_t)strlen("/tmp/sym_target.txt") || strcmp(rl_buf, "/tmp/sym_target.txt") != 0) {
        die("[symlink-test] FAIL: readlink returned unexpected target", 4);
    }
    pmsg("[symlink-test] PASS: readlink() returned exact target string\n");

    struct stat st_link;
    if (lstat("/tmp/sym_link", &st_link) != 0) {
        die("[symlink-test] FAIL: lstat(/tmp/sym_link) failed", 5);
    }
    if (!S_ISLNK(st_link.st_mode)) {
        die("[symlink-test] FAIL: lstat did not report S_IFLNK", 6);
    }

    struct stat st_target;
    if (stat("/tmp/sym_link", &st_target) != 0) {
        die("[symlink-test] FAIL: stat(/tmp/sym_link) failed", 7);
    }
    if (!S_ISREG(st_target.st_mode)) {
        die("[symlink-test] FAIL: stat did not report S_IFREG", 8);
    }
    if (st_target.st_size != (off_t)strlen(orig_data)) {
        die("[symlink-test] FAIL: stat target size mismatch", 9);
    }
    pmsg("[symlink-test] PASS: lstat (S_IFLNK) vs stat (S_IFREG) verified\n");

    fd = open("/tmp/sym_link", O_RDONLY);
    if (fd < 0) {
        die("[symlink-test] FAIL: open(/tmp/sym_link) failed", 10);
    }
    char read_buf[128];
    memset(read_buf, 0, sizeof(read_buf));
    ssize_t bytes_read = read(fd, read_buf, sizeof(read_buf) - 1);
    close(fd);
    if (bytes_read != (ssize_t)strlen(orig_data) || strcmp(read_buf, orig_data) != 0) {
        die("[symlink-test] FAIL: content read through symlink mismatch", 11);
    }
    pmsg("[symlink-test] PASS: bit-for-bit read through symlink verified\n");

    // =========================================================================
    // 2. Cross-mount symlink resolution (Root RedoxFS <-> Secondary tmpfs)
    // =========================================================================
    pmsg("[symlink-test] Test 2: Cross-mount symlink resolution...\n");
    int mrc = syscall(SYS_mount, "none", "/mnt/sym_tmp", "tmpfs", 0, NULL);
    if (mrc != 0) {
        die("[symlink-test] FAIL: mount(/mnt/sym_tmp, tmpfs) failed", 12);
    }
    pmsg("[symlink-test] PASS: mounted secondary tmpfs at /mnt/sym_tmp\n");

    fd = open("/mnt/sym_tmp/cross_secret.txt", O_CREAT | O_WRONLY | O_TRUNC, 0644);
    if (fd < 0) {
        die("[symlink-test] FAIL: create /mnt/sym_tmp/cross_secret.txt failed", 13);
    }
    const char *cross_data = "CrossMountSecretPayload999";
    write(fd, cross_data, strlen(cross_data));
    close(fd);

    unlink("/cross_to_tmp");
    if (symlink("/mnt/sym_tmp/cross_secret.txt", "/cross_to_tmp") != 0) {
        die("[symlink-test] FAIL: symlink on root to tmpfs failed", 14);
    }

    fd = open("/cross_to_tmp", O_RDWR);
    if (fd < 0) {
        die("[symlink-test] FAIL: open /cross_to_tmp failed", 15);
    }
    memset(read_buf, 0, sizeof(read_buf));
    bytes_read = read(fd, read_buf, sizeof(read_buf) - 1);
    if (bytes_read != (ssize_t)strlen(cross_data) || strcmp(read_buf, cross_data) != 0) {
        close(fd);
        die("[symlink-test] FAIL: cross-mount read data mismatch", 16);
    }

    const char *append_data = "-ModifiedAcrossMount";
    lseek(fd, 0, SEEK_END);
    write(fd, append_data, strlen(append_data));
    close(fd);

    fd = open("/mnt/sym_tmp/cross_secret.txt", O_RDONLY);
    if (fd < 0) {
        die("[symlink-test] FAIL: reopen target in tmpfs failed", 17);
    }
    memset(read_buf, 0, sizeof(read_buf));
    bytes_read = read(fd, read_buf, sizeof(read_buf) - 1);
    close(fd);
    char expected_cross[128];
    snprintf(expected_cross, sizeof(expected_cross), "%s%s", cross_data, append_data);
    if (bytes_read != (ssize_t)strlen(expected_cross) || strcmp(read_buf, expected_cross) != 0) {
        die("[symlink-test] FAIL: cross-mount write verification failed", 18);
    }
    pmsg("[symlink-test] PASS: cross-mount symlink read and write verified\n");

    // Reverse cross-mount: symlink in tmpfs pointing back to root filesystem
    unlink("/mnt/sym_tmp/link_to_release");
    if (symlink("/etc/vanta-release", "/mnt/sym_tmp/link_to_release") != 0) {
        die("[symlink-test] FAIL: symlink in tmpfs to root failed", 19);
    }
    fd = open("/mnt/sym_tmp/link_to_release", O_RDONLY);
    if (fd < 0) {
        die("[symlink-test] FAIL: open symlink in tmpfs pointing to root failed", 20);
    }
    memset(read_buf, 0, sizeof(read_buf));
    bytes_read = read(fd, read_buf, sizeof(read_buf) - 1);
    close(fd);
    if (bytes_read <= 0 || strstr(read_buf, "Vanta OS") == NULL) {
        die("[symlink-test] FAIL: reverse cross-mount read did not contain Vanta OS", 21);
    }
    pmsg("[symlink-test] PASS: reverse cross-mount (tmpfs -> root) verified\n");

    // =========================================================================
    // 3. Loop detection: exactly 40 hops SUCCEEDS, exactly 41 hops FAILS (ELOOP)
    // =========================================================================
    pmsg("[symlink-test] Test 3: Loop detection & 40/41 hop limits...\n");
    fd = open("/tmp/hop_target.txt", O_CREAT | O_WRONLY | O_TRUNC, 0644);
    if (fd < 0) {
        die("[symlink-test] FAIL: create /tmp/hop_target.txt failed", 22);
    }
    const char *hop_msg = "ReachedTargetAfter40Hops!";
    write(fd, hop_msg, strlen(hop_msg));
    close(fd);

    // Build chain:
    // hop_link_0 -> /tmp/hop_target.txt
    // hop_link_1 -> hop_link_0
    // ...
    // hop_link_39 -> hop_link_38 (total 40 symlink hops to target)
    // hop_link_40 -> hop_link_39 (total 41 symlink hops to target)
    char link_name[64];
    char target_name[64];
    unlink("/tmp/hop_0");
    if (symlink("/tmp/hop_target.txt", "/tmp/hop_0") != 0) {
        die("[symlink-test] FAIL: symlink hop_0 failed", 23);
    }

    for (int i = 1; i <= 40; i++) {
        snprintf(link_name, sizeof(link_name), "/tmp/hop_%d", i);
        snprintf(target_name, sizeof(target_name), "/tmp/hop_%d", i - 1);
        unlink(link_name);
        if (symlink(target_name, link_name) != 0) {
            die("[symlink-test] FAIL: symlink hop chain creation failed", 24);
        }
    }

    // 40 hops: /tmp/hop_39 -> /tmp/hop_38 -> ... -> /tmp/hop_0 -> /tmp/hop_target.txt
    pmsg("[symlink-test] Testing 40 hops (/tmp/hop_39)...\n");
    fd = open("/tmp/hop_39", O_RDONLY);
    if (fd < 0) {
        pmsg("[symlink-test] errno=");
        char errstr[16];
        snprintf(errstr, sizeof(errstr), "%d", errno);
        pmsg(errstr);
        die(" FAIL: 40-hop symlink resolution failed", 25);
    }
    memset(read_buf, 0, sizeof(read_buf));
    bytes_read = read(fd, read_buf, sizeof(read_buf) - 1);
    close(fd);
    if (bytes_read != (ssize_t)strlen(hop_msg) || strcmp(read_buf, hop_msg) != 0) {
        die("[symlink-test] FAIL: 40-hop read content mismatch", 26);
    }
    pmsg("[symlink-test] PASS: exact 40-hop symlink chain succeeded\n");

    // 41 hops: /tmp/hop_40 -> /tmp/hop_39 -> ... -> hop_0 -> target
    pmsg("[symlink-test] Testing 41 hops (/tmp/hop_40)...\n");
    errno = 0;
    fd = open("/tmp/hop_40", O_RDONLY);
    if (fd >= 0) {
        close(fd);
        die("[symlink-test] FAIL: 41-hop symlink opened successfully (expected ELOOP)", 27);
    }
    if (errno != ELOOP) {
        pmsg("[symlink-test] unexpected errno for 41 hops: ");
        char errstr[16];
        snprintf(errstr, sizeof(errstr), "%d", errno);
        pmsg(errstr);
        die(" (expected ELOOP 40)", 28);
    }
    pmsg("[symlink-test] PASS: exact 41-hop chain returned ELOOP (40)\n");

    // Circular symlink loop: loop_a <-> loop_b
    pmsg("[symlink-test] Testing circular symlink loop...\n");
    unlink("/tmp/loop_a");
    unlink("/tmp/loop_b");
    symlink("/tmp/loop_b", "/tmp/loop_a");
    symlink("/tmp/loop_a", "/tmp/loop_b");
    errno = 0;
    fd = open("/tmp/loop_a", O_RDONLY);
    if (fd >= 0) {
        close(fd);
        die("[symlink-test] FAIL: circular symlink opened successfully", 29);
    }
    if (errno != ELOOP) {
        die("[symlink-test] FAIL: circular symlink did not return ELOOP", 30);
    }
    pmsg("[symlink-test] PASS: circular symlink loop returned ELOOP\n");

    // Broken symlink
    pmsg("[symlink-test] Testing broken symlink...\n");
    unlink("/tmp/broken_link");
    symlink("/tmp/non_existent_file_xyz.txt", "/tmp/broken_link");
    errno = 0;
    fd = open("/tmp/broken_link", O_RDONLY);
    if (fd >= 0) {
        close(fd);
        die("[symlink-test] FAIL: broken symlink opened successfully", 31);
    }
    if (errno != ENOENT) {
        die("[symlink-test] FAIL: broken symlink did not return ENOENT", 32);
    }
    if (lstat("/tmp/broken_link", &st_link) != 0 || !S_ISLNK(st_link.st_mode)) {
        die("[symlink-test] FAIL: lstat on broken symlink failed", 33);
    }
    pmsg("[symlink-test] PASS: broken symlink returned ENOENT on open, S_ISLNK on lstat\n");

    // =========================================================================
    // 4. Dentry cache stress and invalidation
    // =========================================================================
    pmsg("[symlink-test] Test 4: Dentry cache stress & invalidation...\n");
    for (int i = 0; i < 200; i++) {
        if (stat("/cross_to_tmp", &st_target) != 0) {
            die("[symlink-test] FAIL: cached stat on /cross_to_tmp failed", 34);
        }
        if (lstat("/cross_to_tmp", &st_link) != 0) {
            die("[symlink-test] FAIL: cached lstat on /cross_to_tmp failed", 35);
        }
    }
    pmsg("[symlink-test] PASS: 200 repeated lookups hit dentry cache successfully\n");

    // Invalidation on unlink
    if (unlink("/mnt/sym_tmp/cross_secret.txt") != 0) {
        die("[symlink-test] FAIL: unlink target in tmpfs failed", 36);
    }
    errno = 0;
    fd = open("/cross_to_tmp", O_RDONLY);
    if (fd >= 0) {
        close(fd);
        die("[symlink-test] FAIL: open unlinked target through symlink succeeded", 37);
    }
    if (errno != ENOENT) {
        die("[symlink-test] FAIL: open unlinked target did not return ENOENT", 38);
    }
    unlink("/cross_to_tmp");
    errno = 0;
    if (lstat("/cross_to_tmp", &st_link) == 0 || errno != ENOENT) {
        die("[symlink-test] FAIL: lstat unlinked symlink did not return ENOENT", 39);
    }
    pmsg("[symlink-test] PASS: dentry cache invalidation on unlink verified\n");

    // Unmount temporary mount
    syscall(SYS_umount2, "/mnt/sym_tmp", 0);

    pmsg("[symlink-test] ALL TESTS PASSED SUCCESSFULLY (Exit Code 0)\n");
    return 0;
}
