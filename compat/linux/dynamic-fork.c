#define _GNU_SOURCE
#include <unistd.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <stdio.h>
#include <stdlib.h>

static int shared_val = 100;

static __attribute__((noinline)) int recurse_stack(int depth, int acc) {
    volatile char frame_buf[4096];
    frame_buf[0] = (char)(depth + 1);
    frame_buf[4095] = (char)(depth ^ 0x55);
    acc += (int)frame_buf[0] + (int)frame_buf[4095];
    if (depth <= 0) {
        return acc;
    }
    return recurse_stack(depth - 1, acc);
}

int main(void) {
    // Phase 1: Rapid 50-iteration fork loop stress test
    for (int i = 0; i < 50; i++) {
        pid_t p = fork();
        if (p < 0) {
            printf("[linux-fork] fork failed at iteration %d\n", i);
            return 10;
        }
        if (p == 0) {
            // Child: touch memory, verify isolation, exit with distinct status
            volatile char scratch[256];
            scratch[0] = (char)(i + 7);
            scratch[255] = (char)(i * 3);
            _exit((i + 1) % 100);
        }
        int status = 0;
        pid_t w = waitpid(p, &status, 0);
        if (w != p || !WIFEXITED(status) || WEXITSTATUS(status) != ((i + 1) % 100)) {
            printf("[linux-fork] waitpid failed at iter %d: w=%d status=%d\n", i, (int)w, WEXITSTATUS(status));
            return 11;
        }
    }
    printf("[linux-fork] 50-iteration fork loop verified\n");

    // Phase 2: Single fork COW / memory isolation check
    pid_t pid = fork();
    if (pid < 0) {
        printf("[linux-fork] fork failed\n");
        return 1;
    }
    if (pid == 0) {
        // In child
        shared_val += 50;
        printf("[linux-fork] child executed shared_val=%d\n", shared_val);
        _exit(42);
    } else {
        // In parent
        int status = 0;
        pid_t w = waitpid(pid, &status, 0);
        printf("[linux-fork] parent waited w=%d status=%d shared_val=%d\n", (int)w, WEXITSTATUS(status), shared_val);
        if (shared_val != 100 || WEXITSTATUS(status) != 42) {
            return 2;
        }
        printf("[linux-fork] COW fork and waitpid verified\n");
    }

    // Phase 2b: Invalid user memory access (null pointer dereference) termination check
    pid_t bad_child = fork();
    if (bad_child < 0) {
        printf("[linux-fork] bad_child fork failed\n");
        return 30;
    }
    if (bad_child == 0) {
        // Child intentionally writes to null pointer to trigger invalid access
        volatile int *bad_ptr = (volatile int *)0x0;
        *bad_ptr = 0xdead;
        _exit(0);
    }
    int bad_status = 0;
    pid_t bad_w = waitpid(bad_child, &bad_status, 0);
    if (bad_w == bad_child && WEXITSTATUS(bad_status) == 139) {
        printf("[linux-fork] invalid memory access SIGSEGV termination verified\n");
    } else {
        printf("[linux-fork] invalid memory test unexpected: w=%d status=%d\n", (int)bad_w, WEXITSTATUS(bad_status));
        return 31;
    }

    // Phase 3: Stack auto-expansion beyond initial 64KB stack limit
    int stack_res = recurse_stack(48, 0);
    if (stack_res != 0) {
        printf("[linux-fork] stack auto-expansion verified\n");
    }

    // Phase 4: Anonymous demand allocation test
    char *sparse = malloc(2 * 1024 * 1024);
    if (sparse != NULL) {
        sparse[0] = 'V';
        sparse[1024 * 1024] = 'A';
        sparse[2 * 1024 * 1024 - 1] = 'N';
        if (sparse[0] == 'V' && sparse[1024 * 1024] == 'A' && sparse[2 * 1024 * 1024 - 1] == 'N') {
            printf("[linux-fork] anonymous demand paging verified\n");
        }
        free(sparse);
    }

    return 0;
}
