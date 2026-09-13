#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <termios.h>
#include <unistd.h>

#ifndef TIOCGPTN
#define TIOCGPTN 0x80045430
#endif
#ifndef TIOCSPTLCK
#define TIOCSPTLCK 0x40045431
#endif

static volatile sig_atomic_t g_got_sigint = 0;
static volatile sig_atomic_t g_got_sigwinch = 0;

static void handle_sigint(int sig) {
    (void)sig;
    g_got_sigint = 1;
}

static void handle_sigwinch(int sig) {
    (void)sig;
    g_got_sigwinch = 1;
}

int main(int argc, char **argv) {
    (void)argc;
    (void)argv;
    printf("[pty-test] starting Pseudo-Terminal Subsystem Test (Phase 4 Unit 6)...\n");

    // Test 1: Open /dev/ptmx and acquire slave index
    printf("[pty-test] Test 1: Opening /dev/ptmx and querying TIOCGPTN / TIOCSPTLCK...\n");
    int master_fd = open("/dev/ptmx", O_RDWR);
    if (master_fd < 0) {
        printf("[pty-test] FAIL: open(/dev/ptmx) failed: %d\n", errno);
        return 1;
    }

    int pty_num = -1;
    if (ioctl(master_fd, TIOCGPTN, &pty_num) < 0) {
        printf("[pty-test] FAIL: ioctl(TIOCGPTN) failed: %d\n", errno);
        close(master_fd);
        return 2;
    }
    printf("[pty-test] Allocated PTY slave index: %d\n", pty_num);

    int unlock = 0;
    if (ioctl(master_fd, TIOCSPTLCK, &unlock) < 0) {
        printf("[pty-test] FAIL: ioctl(TIOCSPTLCK) failed: %d\n", errno);
        close(master_fd);
        return 3;
    }

    char pts_path[64];
    snprintf(pts_path, sizeof(pts_path), "/dev/pts/%d", pty_num);
    int slave_fd = open(pts_path, O_RDWR);
    if (slave_fd < 0) {
        printf("[pty-test] FAIL: open(%s) failed: %d\n", pts_path, errno);
        close(master_fd);
        return 4;
    }
    printf("[pty-test] PASS: Master/slave pair opened successfully\n");

    // Test 2: Canonical line discipline & backspace editing
    printf("[pty-test] Test 2: Canonical line discipline and backspace editing...\n");
    // Write "helloo\x08 world\n" to master
    const char *edit_input = "helloo\x08 world\n";
    ssize_t w = write(master_fd, edit_input, strlen(edit_input));
    if (w != (ssize_t)strlen(edit_input)) {
        printf("[pty-test] FAIL: write to master_fd failed: %d\n", errno);
        close(master_fd);
        close(slave_fd);
        return 5;
    }

    char read_buf[128];
    memset(read_buf, 0, sizeof(read_buf));
    ssize_t r = read(slave_fd, read_buf, sizeof(read_buf) - 1);
    if (r <= 0) {
        printf("[pty-test] FAIL: read from slave_fd failed: %d\n", errno);
        close(master_fd);
        close(slave_fd);
        return 6;
    }
    read_buf[r] = '\0';
    printf("[pty-test] Slave read: '%s'\n", read_buf);
    if (strcmp(read_buf, "hello world\n") != 0) {
        printf("[pty-test] FAIL: expected 'hello world\\n', got '%s'\n", read_buf);
        close(master_fd);
        close(slave_fd);
        return 7;
    }
    printf("[pty-test] PASS: Canonical line discipline & backspace editing verified\n");

    // Test 3: Raw mode pass-through
    printf("[pty-test] Test 3: Raw mode pass-through...\n");
    struct termios tio;
    if (tcgetattr(slave_fd, &tio) < 0) {
        printf("[pty-test] FAIL: tcgetattr failed: %d\n", errno);
        close(master_fd);
        close(slave_fd);
        return 8;
    }
    tio.c_lflag &= ~ICANON; // Disable canonical mode
    tio.c_lflag &= ~ECHO;   // Disable echo
    if (tcsetattr(slave_fd, TCSANOW, &tio) < 0) {
        printf("[pty-test] FAIL: tcsetattr failed: %d\n", errno);
        close(master_fd);
        close(slave_fd);
        return 9;
    }

    // Write bytes without newline in raw mode
    const char *raw_data = "ABC";
    write(master_fd, raw_data, 3);
    memset(read_buf, 0, sizeof(read_buf));
    r = read(slave_fd, read_buf, 3);
    if (r != 3 || memcmp(read_buf, "ABC", 3) != 0) {
        printf("[pty-test] FAIL: raw mode pass-through failed, got r=%zd\n", r);
        close(master_fd);
        close(slave_fd);
        return 10;
    }
    printf("[pty-test] PASS: Raw mode pass-through verified\n");

    // Restore canonical mode
    tio.c_lflag |= ICANON | ECHO;
    tcsetattr(slave_fd, TCSANOW, &tio);

    // Test 4: Window resizing (TIOCSWINSZ) and SIGWINCH injection
    printf("[pty-test] Test 4: TIOCSWINSZ / SIGWINCH...\n");
    signal(SIGWINCH, handle_sigwinch);
    // Set controlling terminal and foreground pgrp to current process
    ioctl(slave_fd, TIOCSCTTY, 0);
    pid_t my_pgrp = getpgrp();
    ioctl(slave_fd, TIOCSPGRP, &my_pgrp);

    struct winsize ws;
    memset(&ws, 0, sizeof(ws));
    ws.ws_row = 40;
    ws.ws_col = 120;
    if (ioctl(master_fd, TIOCSWINSZ, &ws) < 0) {
        printf("[pty-test] FAIL: ioctl(TIOCSWINSZ) failed: %d\n", errno);
        close(master_fd);
        close(slave_fd);
        return 11;
    }

    struct winsize check_ws;
    memset(&check_ws, 0, sizeof(check_ws));
    if (ioctl(slave_fd, TIOCGWINSZ, &check_ws) < 0) {
        printf("[pty-test] FAIL: ioctl(TIOCGWINSZ) failed: %d\n", errno);
        close(master_fd);
        close(slave_fd);
        return 12;
    }
    if (check_ws.ws_row != 40 || check_ws.ws_col != 120) {
        printf("[pty-test] FAIL: winsize mismatch: %dx%d != 40x120\n", check_ws.ws_row, check_ws.ws_col);
        close(master_fd);
        close(slave_fd);
        return 13;
    }
    if (!g_got_sigwinch) {
        printf("[pty-test] FAIL: did not receive SIGWINCH on TIOCSWINSZ\n");
        close(master_fd);
        close(slave_fd);
        return 14;
    }
    printf("[pty-test] PASS: TIOCSWINSZ updated winsize and injected SIGWINCH\n");

    // Test 5: Ctrl+C (0x03) signal generation to foreground process
    printf("[pty-test] Test 5: Ctrl+C (0x03) -> SIGINT signal generation...\n");
    signal(SIGINT, handle_sigint);
    char ctrl_c = 0x03;
    write(master_fd, &ctrl_c, 1);
    if (!g_got_sigint) {
        printf("[pty-test] FAIL: did not receive SIGINT on Ctrl+C\n");
        close(master_fd);
        close(slave_fd);
        return 15;
    }
    printf("[pty-test] PASS: Ctrl+C injected SIGINT into foreground process group\n");

    // Test 6: Master close produces EOF on slave
    printf("[pty-test] Test 6: Master close producing EOF on slave...\n");
    close(master_fd);
    memset(read_buf, 0, sizeof(read_buf));
    r = read(slave_fd, read_buf, sizeof(read_buf));
    if (r != 0) {
        printf("[pty-test] FAIL: read on closed master did not return 0 (got %zd)\n", r);
        close(slave_fd);
        return 16;
    }
    close(slave_fd);
    printf("[pty-test] PASS: Master close produced EOF on slave read\n");

    printf("[pty-test] ALL TESTS PASSED SUCCESSFULLY (Exit Code 0)\n");
    return 0;
}
