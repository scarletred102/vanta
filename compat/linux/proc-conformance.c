#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

int main(int argc, char **argv) {
    (void)argc;
    (void)argv;
    printf("[proc-conformance] starting Synthetic Filesystems Conformance Test (Phase 4 Unit 5)...\n");

    // Test 1: /proc/self/exe
    printf("[proc-conformance] Test 1: Reading /proc/self/exe...\n");
    char exe_buf[256];
    memset(exe_buf, 0, sizeof(exe_buf));
    ssize_t exe_len = readlink("/proc/self/exe", exe_buf, sizeof(exe_buf) - 1);
    if (exe_len <= 0) {
        printf("[proc-conformance] FAIL: readlink(/proc/self/exe) failed: %d\n", errno);
        return 1;
    }
    exe_buf[exe_len] = '\0';
    printf("[proc-conformance] /proc/self/exe -> %s\n", exe_buf);
    if (strstr(exe_buf, "proc-conformance") == NULL) {
        printf("[proc-conformance] FAIL: exe path does not contain 'proc-conformance'\n");
        return 2;
    }
    // Verify opening through symlink
    int exe_fd = open("/proc/self/exe", O_RDONLY);
    if (exe_fd < 0) {
        printf("[proc-conformance] FAIL: open(/proc/self/exe) failed: %d\n", errno);
        return 3;
    }
    char elf_magic[4];
    if (read(exe_fd, elf_magic, 4) != 4 || memcmp(elf_magic, "\x7f\x45\x4c\x46", 4) != 0) {
        printf("[proc-conformance] FAIL: reading /proc/self/exe did not return ELF header\n");
        close(exe_fd);
        return 4;
    }
    close(exe_fd);
    printf("[proc-conformance] PASS: /proc/self/exe resolves to valid ELF binary\n");

    // Test 2: /proc/self/maps
    printf("[proc-conformance] Test 2: Reading /proc/self/maps...\n");
    int maps_fd = open("/proc/self/maps", O_RDONLY);
    if (maps_fd < 0) {
        printf("[proc-conformance] FAIL: open(/proc/self/maps) failed: %d\n", errno);
        return 5;
    }
    char maps_buf[4096];
    memset(maps_buf, 0, sizeof(maps_buf));
    ssize_t maps_len = read(maps_fd, maps_buf, sizeof(maps_buf) - 1);
    close(maps_fd);
    if (maps_len <= 0) {
        printf("[proc-conformance] FAIL: read(/proc/self/maps) returned %zd, errno=%d\n", maps_len, errno);
        return 6;
    }
    maps_buf[maps_len] = '\0';
    if (strstr(maps_buf, "[stack]") == NULL) {
        printf("[proc-conformance] FAIL: /proc/self/maps does not contain [stack]\n");
        return 7;
    }
    if (strstr(maps_buf, "r-xp") == NULL && strstr(maps_buf, "r--p") == NULL) {
        printf("[proc-conformance] FAIL: /proc/self/maps does not contain valid permissions\n");
        return 8;
    }
    printf("[proc-conformance] PASS: /proc/self/maps displays valid VMA layout\n");

    // Test 3: /dev/null
    printf("[proc-conformance] Test 3: Testing /dev/null...\n");
    int null_fd = open("/dev/null", O_RDWR);
    if (null_fd < 0) {
        printf("[proc-conformance] FAIL: open(/dev/null) failed: %d\n", errno);
        return 9;
    }
    char dummy[1024];
    memset(dummy, 'A', sizeof(dummy));
    ssize_t written = write(null_fd, dummy, sizeof(dummy));
    if (written != (ssize_t)sizeof(dummy)) {
        printf("[proc-conformance] FAIL: write to /dev/null returned %zd (expected %zu)\n", written, sizeof(dummy));
        close(null_fd);
        return 10;
    }
    ssize_t read_bytes = read(null_fd, dummy, sizeof(dummy));
    if (read_bytes != 0) {
        printf("[proc-conformance] FAIL: read from /dev/null returned %zd (expected 0 EOF)\n", read_bytes);
        close(null_fd);
        return 11;
    }
    close(null_fd);
    printf("[proc-conformance] PASS: /dev/null discards writes and returns EOF on read\n");

    // Test 4: /dev/zero
    printf("[proc-conformance] Test 4: Testing /dev/zero...\n");
    int zero_fd = open("/dev/zero", O_RDONLY);
    if (zero_fd < 0) {
        printf("[proc-conformance] FAIL: open(/dev/zero) failed: %d\n", errno);
        return 12;
    }
    memset(dummy, 0xFF, sizeof(dummy));
    read_bytes = read(zero_fd, dummy, sizeof(dummy));
    close(zero_fd);
    if (read_bytes != (ssize_t)sizeof(dummy)) {
        printf("[proc-conformance] FAIL: read from /dev/zero returned %zd\n", read_bytes);
        return 13;
    }
    for (size_t i = 0; i < sizeof(dummy); i++) {
        if (dummy[i] != 0) {
            printf("[proc-conformance] FAIL: /dev/zero byte %zu is non-zero (0x%02x)\n", i, (unsigned char)dummy[i]);
            return 14;
        }
    }
    printf("[proc-conformance] PASS: /dev/zero returns continuous 0x00 bytes\n");

    // Test 5: /dev/urandom
    printf("[proc-conformance] Test 5: Testing /dev/urandom...\n");
    int urand_fd = open("/dev/urandom", O_RDONLY);
    if (urand_fd < 0) {
        printf("[proc-conformance] FAIL: open(/dev/urandom) failed: %d\n", errno);
        return 15;
    }
    uint8_t rand1[32];
    uint8_t rand2[32];
    if (read(urand_fd, rand1, sizeof(rand1)) != sizeof(rand1) ||
        read(urand_fd, rand2, sizeof(rand2)) != sizeof(rand2)) {
        printf("[proc-conformance] FAIL: failed to read entropy from /dev/urandom\n");
        close(urand_fd);
        return 16;
    }
    close(urand_fd);
    if (memcmp(rand1, rand2, sizeof(rand1)) == 0) {
        printf("[proc-conformance] FAIL: /dev/urandom produced identical buffers (deterministic!)\n");
        return 17;
    }
    printf("[proc-conformance] PASS: /dev/urandom furnishes non-deterministic CSPRNG entropy\n");

    // Test 6: System telemetry files
    printf("[proc-conformance] Test 6: Verifying /proc/cpuinfo, /proc/meminfo, /proc/uptime...\n");
    int cpu_fd = open("/proc/cpuinfo", O_RDONLY);
    if (cpu_fd < 0) {
        printf("[proc-conformance] FAIL: open(/proc/cpuinfo) failed: %d\n", errno);
        return 18;
    }
    char telem[1024];
    ssize_t telem_len = read(cpu_fd, telem, sizeof(telem) - 1);
    close(cpu_fd);
    if (telem_len <= 0 || strstr(telem, "processor") == NULL) {
        printf("[proc-conformance] FAIL: /proc/cpuinfo content invalid\n");
        return 19;
    }

    int mem_fd = open("/proc/meminfo", O_RDONLY);
    if (mem_fd < 0) {
        printf("[proc-conformance] FAIL: open(/proc/meminfo) failed: %d\n", errno);
        return 20;
    }
    telem_len = read(mem_fd, telem, sizeof(telem) - 1);
    close(mem_fd);
    if (telem_len <= 0 || strstr(telem, "MemTotal") == NULL) {
        printf("[proc-conformance] FAIL: /proc/meminfo content invalid\n");
        return 21;
    }

    int upt_fd = open("/proc/uptime", O_RDONLY);
    if (upt_fd < 0) {
        printf("[proc-conformance] FAIL: open(/proc/uptime) failed: %d\n", errno);
        return 22;
    }
    telem_len = read(upt_fd, telem, sizeof(telem) - 1);
    close(upt_fd);
    if (telem_len <= 0) {
        printf("[proc-conformance] FAIL: /proc/uptime content empty\n");
        return 23;
    }
    printf("[proc-conformance] PASS: system telemetry streams furnish accurate runtime stats\n");

    printf("[proc-conformance] ALL TESTS PASSED SUCCESSFULLY (Exit Code 0)\n");
    return 0;
}