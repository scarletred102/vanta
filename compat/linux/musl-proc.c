#include <unistd.h>
#include <sys/utsname.h>
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

    static const char msg[] = "[linux-musl] posix system info passed\n";
    write(1, msg, sizeof(msg) - 1);
    return 0;
}
