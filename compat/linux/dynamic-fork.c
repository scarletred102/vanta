#define _GNU_SOURCE
#include <unistd.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <stdio.h>
#include <stdlib.h>

static int shared_val = 100;

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
        if (shared_val == 100 && WEXITSTATUS(status) == 42) {
            printf("[linux-fork] COW fork and waitpid verified\n");
            return 0;
        }
        return 2;
    }
}
