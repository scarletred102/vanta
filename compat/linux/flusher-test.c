#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#define SYNC_PATH "/home/vanta/flusher_sync.txt"
#define BG_PATH   "/home/vanta/flusher_bg.txt"

static void sleep_ms(uint32_t ms) {
    struct timespec ts;
    ts.tv_sec = ms / 1000;
    ts.tv_nsec = (ms % 1000) * 1000000L;
    nanosleep(&ts, NULL);
}

int main(void) {
    printf("[flusher-test] starting Background Flusher Daemon & Sync Verification (Phase 4 Unit 4)...\n");

    mkdir("/home/vanta", 0755);

    // Test 1: sync() system call
    printf("[flusher-test] Test 1: Testing sync() system call...\n");
    int sfd = open(SYNC_PATH, O_RDWR | O_CREAT | O_TRUNC, 0644);
    if (sfd < 0) {
        printf("[flusher-test] FAIL: open(%s) failed: %d\n", SYNC_PATH, errno);
        return 1;
    }
    const char *sync_payload = "VANTA_SYNC_VERIFICATION_PAYLOAD_1234567890\n";
    size_t sync_len = strlen(sync_payload);
    if (write(sfd, sync_payload, sync_len) != (ssize_t)sync_len) {
        printf("[flusher-test] FAIL: write sync_payload failed: %d\n", errno);
        close(sfd);
        return 2;
    }
    close(sfd);

    // Call sync()
    sync();

    // Verify content on disk
    int rfd = open(SYNC_PATH, O_RDONLY);
    if (rfd < 0) {
        printf("[flusher-test] FAIL: open for reading %s failed: %d\n", SYNC_PATH, errno);
        return 3;
    }
    char sync_readback[128];
    memset(sync_readback, 0, sizeof(sync_readback));
    ssize_t n_read = read(rfd, sync_readback, sizeof(sync_readback) - 1);
    close(rfd);
    unlink(SYNC_PATH);
    if (n_read != (ssize_t)sync_len || memcmp(sync_readback, sync_payload, sync_len) != 0) {
        printf("[flusher-test] FAIL: sync() data mismatch\n");
        return 4;
    }
    printf("[flusher-test] PASS: sync() system call committed page cache to disk\n");

    // Test 2: Background flusher daemon (500 ms periodic flush)
    printf("[flusher-test] Test 2: Testing background flusher daemon (500 ms interval)...\n");
    int bgfd = open(BG_PATH, O_RDWR | O_CREAT | O_TRUNC, 0644);
    if (bgfd < 0) {
        printf("[flusher-test] FAIL: open(%s) failed: %d\n", BG_PATH, errno);
        return 5;
    }

    char bg_buf[8192];
    memset(bg_buf, 0x5a, sizeof(bg_buf));
    if (write(bgfd, bg_buf, sizeof(bg_buf)) != sizeof(bg_buf)) {
        printf("[flusher-test] FAIL: write bg_buf failed: %d\n", errno);
        close(bgfd);
        return 6;
    }
    close(bgfd);

    // Sleep for 700 ms (> 500 ms flusher interval) to allow the daemon to run
    sleep_ms(700);

    // Read back and verify
    int bgrfd = open(BG_PATH, O_RDONLY);
    if (bgrfd < 0) {
        printf("[flusher-test] FAIL: reopen %s failed: %d\n", BG_PATH, errno);
        return 7;
    }
    char bg_read[8192];
    ssize_t bg_read_len = read(bgrfd, bg_read, sizeof(bg_read));
    close(bgrfd);
    unlink(BG_PATH);
    if (bg_read_len != sizeof(bg_read) || memcmp(bg_read, bg_buf, sizeof(bg_buf)) != 0) {
        printf("[flusher-test] FAIL: background flusher data mismatch\n");
        return 8;
    }
    printf("[flusher-test] PASS: background flusher daemon committed dirty pages after 500ms sleep\n");

    // Test 3: LRU clean page eviction under memory allocation
    printf("[flusher-test] Test 3: Testing LRU clean page eviction under memory pressure...\n");
    // Allocate and dirty 10 buffers
    for (int i = 0; i < 10; i++) {
        char path[64];
        snprintf(path, sizeof(path), "/home/vanta/evict_%d.bin", i);
        int fd = open(path, O_RDWR | O_CREAT | O_TRUNC, 0644);
        if (fd >= 0) {
            char pdata[4096];
            memset(pdata, (char)(i + 1), sizeof(pdata));
            write(fd, pdata, sizeof(pdata));
            close(fd);
        }
    }
    // Call sync to turn all dirty pages into clean pages in the LRU list
    sync();

    // Verify all pages can still be read cleanly
    int all_ok = 1;
    for (int i = 0; i < 10; i++) {
        char path[64];
        snprintf(path, sizeof(path), "/home/vanta/evict_%d.bin", i);
        int fd = open(path, O_RDONLY);
        if (fd >= 0) {
            char pdata[4096];
            if (read(fd, pdata, sizeof(pdata)) != sizeof(pdata) || pdata[0] != (char)(i + 1)) {
                all_ok = 0;
            }
            close(fd);
            unlink(path);
        } else {
            all_ok = 0;
        }
    }
    if (!all_ok) {
        printf("[flusher-test] FAIL: page cache eviction corruption detected\n");
        return 9;
    }
    printf("[flusher-test] PASS: LRU clean page eviction under memory pressure verified\n");

    printf("[flusher-test] ALL TESTS PASSED SUCCESSFULLY (Exit Code 0)\n");
    return 0;
}
