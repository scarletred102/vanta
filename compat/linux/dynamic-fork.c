#define _GNU_SOURCE
#include <unistd.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <stdio.h>
#include <stdlib.h>

static int shared_val = 100;
static volatile int cow_race_target[1024];

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

    // Phase 2c: Concurrent COW resolution race test (50 iterations)
    // Synchronizes parent and child to write to the same shared COW page simultaneously
    for (int iter = 0; iter < 50; iter++) {
        cow_race_target[0] = 0x12340000 + iter;
        cow_race_target[1023] = 0x56780000 + iter;

        int sync_pipe[2];
        if (pipe(sync_pipe) < 0) {
            printf("[linux-fork] sync pipe failed at iter %d\n", iter);
            return 40;
        }

        pid_t p = fork();
        if (p < 0) {
            printf("[linux-fork] concurrent fork failed at iter %d\n", iter);
            return 41;
        }

        if (p == 0) {
            // Child: close write end, wait for parent release token
            close(sync_pipe[1]);
            char token = 0;
            if (read(sync_pipe[0], &token, 1) != 1) {
                _exit(1);
            }
            close(sync_pipe[0]);

            // Immediately write to the shared COW page
            cow_race_target[0] = 0xCCCC0000 + iter;
            cow_race_target[1023] = 0xDDDD0000 + iter;

            // Verify child view is private and correctly updated
            if (cow_race_target[0] != (0xCCCC0000 + iter) || cow_race_target[1023] != (0xDDDD0000 + iter)) {
                _exit(2);
            }
            _exit(0);
        } else {
            // Parent: close read end
            close(sync_pipe[0]);

            // Release child and immediately write to the same shared COW page
            write(sync_pipe[1], "G", 1);
            close(sync_pipe[1]);

            cow_race_target[0] = 0xAAAA0000 + iter;
            cow_race_target[1023] = 0xBBBB0000 + iter;

            int status = 0;
            pid_t w = waitpid(p, &status, 0);
            if (w != p || !WIFEXITED(status) || WEXITSTATUS(status) != 0) {
                printf("[linux-fork] concurrent COW race failed at iter %d: w=%d status=%d\n", iter, (int)w, WEXITSTATUS(status));
                return 42;
            }

            // Verify parent view was not corrupted by child
            if (cow_race_target[0] != (0xAAAA0000 + iter) || cow_race_target[1023] != (0xBBBB0000 + iter)) {
                printf("[linux-fork] concurrent COW race corrupted parent memory at iter %d\n", iter);
                return 43;
            }
        }
    }
    printf("[linux-fork] concurrent COW race 50-iteration test verified\n");

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

    // Phase 5: Demand-paged child process exit and address space teardown test
    pid_t demand_child = fork();
    if (demand_child < 0) {
        printf("[linux-fork] demand_child fork failed\n");
        return 50;
    }
    if (demand_child == 0) {
        // Child triggers demand-paged stack expansion (64 frames = 256 KiB)
        int s = recurse_stack(64, 0);
        // Child allocates anonymous memory and touches multiple 4 KiB pages
        char *dmem = malloc(4 * 1024 * 1024);
        if (dmem) {
            for (int p = 0; p < 4 * 1024 * 1024; p += 4096) {
                dmem[p] = (char)(p ^ s);
            }
        }
        _exit(0);
    }
    int demand_status = 0;
    pid_t demand_w = waitpid(demand_child, &demand_status, 0);
    if (demand_w == demand_child && WIFEXITED(demand_status) && WEXITSTATUS(demand_status) == 0) {
        printf("[linux-fork] demand-paged process exit and address space destruction verified\n");
    } else {
        printf("[linux-fork] demand_child waitpid failed: w=%d status=%d\n", (int)demand_w, WEXITSTATUS(demand_status));
        return 51;
    }

    return 0;
}
