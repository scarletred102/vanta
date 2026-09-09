#include <unistd.h>
#include <sys/utsname.h>
#include <sys/syscall.h>
#include <time.h>
#include <string.h>

int main(void) {
    // 1. uname
    struct utsname uts;
    if (uname(&uts) != 0) {
        return 1;
    }
    if (strcmp(uts.sysname, "Linux") != 0) {
        return 2;
    }

    // 2. getcwd
    char cwd[128];
    if (!getcwd(cwd, sizeof(cwd))) {
        return 3;
    }
    if (cwd[0] != '/') {
        return 4;
    }

    // 3. clock_gettime
    struct timespec ts;
    if (clock_gettime(CLOCK_REALTIME, &ts) != 0) {
        return 5;
    }
    if (ts.tv_sec <= 0) {
        return 6;
    }

    // 4. getpid
    pid_t pid = getpid();
    if (pid <= 0) {
        return 7;
    }

    // 5. Process groups & session tests
    pid_t pgrp = getpgrp();
    if (pgrp <= 0) {
        return 10;
    }
    if (setpgid(0, 0) != 0) {
        return 11;
    }
    if (getpgrp() != pid) {
        return 12;
    }
    pid_t sid = getsid(0);
    if (sid <= 0) {
        return 13;
    }

    // 6. Termios window size ioctl check
    struct {
        unsigned short ws_row;
        unsigned short ws_col;
        unsigned short ws_xpixel;
        unsigned short ws_ypixel;
    } ws;
    memset(&ws, 0, sizeof(ws));
    // TIOCGWINSZ = 0x5413
    if (syscall(SYS_ioctl, 0, 0x5413, &ws) != 0) {
        return 14;
    }
    if (ws.ws_row != 24 || ws.ws_col != 80) {
        return 15;
    }

    // 7. Bad pointer & invalid syscall negative tests (kernel must not panic)
    if (read(0, (void*)0x0, 16) >= 0) {
        return 16;
    }
    if (write(1, (void*)0xffff800000000000ULL, 16) >= 0) {
        return 17;
    }
    if (read(0, (void*)0x00007ffffffffff0ULL, 4096) >= 0) {
        return 18;
    }
    long bad_sys = syscall(99999);
    if (bad_sys >= 0) {
        return 19;
    }

    static const char msg1[] = "[linux-musl] process groups and termios ioctls verified\n";
    write(1, msg1, sizeof(msg1) - 1);
    static const char msg2[] = "[linux-musl] bad pointer negative tests verified\n";
    write(1, msg2, sizeof(msg2) - 1);
    static const char msg[] = "[linux-musl] posix system info passed\n";
    write(1, msg, sizeof(msg) - 1);
    return 0;
}
