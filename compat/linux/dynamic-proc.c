#define _GNU_SOURCE
#include <fcntl.h>
#include <unistd.h>
#include <stdio.h>
#include <string.h>

int main(void) {
    char buf[512];
    int fd = open("/proc/cpuinfo", O_RDONLY);
    if (fd < 0) {
        printf("[linux-proc] open /proc/cpuinfo failed\n");
        return 1;
    }
    ssize_t n = read(fd, buf, sizeof(buf) - 1);
    close(fd);
    if (n <= 0) {
        printf("[linux-proc] read /proc/cpuinfo failed\n");
        return 2;
    }
    buf[n] = '\0';
    if (!strstr(buf, "processor")) {
        printf("[linux-proc] /proc/cpuinfo missing processor header\n");
        return 3;
    }

    fd = open("/proc/self/status", O_RDONLY);
    if (fd < 0) {
        printf("[linux-proc] open /proc/self/status failed\n");
        return 4;
    }
    n = read(fd, buf, sizeof(buf) - 1);
    close(fd);
    if (n <= 0) {
        printf("[linux-proc] read /proc/self/status failed\n");
        return 5;
    }
    buf[n] = '\0';
    if (!strstr(buf, "Pid:")) {
        printf("[linux-proc] /proc/self/status missing Pid field\n");
        return 6;
    }

    printf("[linux-proc] /proc virtual filesystem verified\n");
    return 0;
}
