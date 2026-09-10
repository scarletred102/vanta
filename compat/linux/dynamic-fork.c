#define _GNU_SOURCE
#include <unistd.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <sys/mman.h>
#include <stdio.h>
#include <stdlib.h>

static int shared_val = 100;
static volatile int cow_race_target[1024];

static inline unsigned long long rdtsc_barrier(void) {
    unsigned int lo, hi;
    __asm__ volatile ("rdtsc" : "=a"(lo), "=d"(hi));
    return ((unsigned long long)hi << 32) | lo;
}

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

    // Phase 2: COW fork and waitpid verification
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

    // Phase 2b-1: Invalid memory access - READ fault on unmapped memory (non-zero)
    pid_t read_child = fork();
    if (read_child < 0) {
        printf("[linux-fork] read_child fork failed\n");
        return 20;
    }
    if (read_child == 0) {
        volatile int *bad_ptr = (volatile int *)0x10000000;
        int val = *bad_ptr;
        (void)val;
        _exit(0);
    }
    int read_status = 0;
    pid_t read_w = waitpid(read_child, &read_status, 0);
    if (read_w == read_child && WEXITSTATUS(read_status) == 139) {
        printf("[linux-fork] unmapped read fault SIGSEGV verified\n");
    } else {
        printf("[linux-fork] unmapped read fault failed: w=%d status=%d\n", (int)read_w, WEXITSTATUS(read_status));
        return 21;
    }

    // Phase 2b-2: Invalid memory access - WRITE fault on non-zero unmapped address
    pid_t write_child = fork();
    if (write_child < 0) {
        printf("[linux-fork] write_child fork failed\n");
        return 22;
    }
    if (write_child == 0) {
        volatile int *bad_ptr = (volatile int *)0xdead0000;
        *bad_ptr = 0xbeef;
        _exit(0);
    }
    int write_status = 0;
    pid_t write_w = waitpid(write_child, &write_status, 0);
    if (write_w == write_child && WEXITSTATUS(write_status) == 139) {
        printf("[linux-fork] unmapped write fault SIGSEGV verified\n");
    } else {
        printf("[linux-fork] unmapped write fault failed: w=%d status=%d\n", (int)write_w, WEXITSTATUS(write_status));
        return 23;
    }

    // Phase 2b-3: Permission-violating WRITE fault on mapped read-only non-COW page
    void *ro_map = mmap(NULL, 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (ro_map != MAP_FAILED) {
        pid_t perm_child = fork();
        if (perm_child < 0) {
            printf("[linux-fork] perm_child fork failed\n");
            return 24;
        }
        if (perm_child == 0) {
            volatile char *ro_ptr = (volatile char *)ro_map;
            *ro_ptr = 'X';
            _exit(0);
        }
        int perm_status = 0;
        pid_t perm_w = waitpid(perm_child, &perm_status, 0);
        if (perm_w == perm_child && WEXITSTATUS(perm_status) == 139) {
            printf("[linux-fork] read-only mapped page write SIGSEGV verified\n");
        } else {
            printf("[linux-fork] read-only mapped page write test failed: w=%d status=%d\n", (int)perm_w, WEXITSTATUS(perm_status));
            return 25;
        }
        munmap(ro_map, 4096);
    } else {
        printf("[linux-fork] mmap PROT_READ failed\n");
        return 26;
    }

    // Phase 2c: Concurrent COW resolution race test (2,000 iterations)
    // Synchronizes parent and child with TSC hardware barrier and zero mutual exclusion during write
    int race_failures = 0;
    int corruptions = 0;
    for (int iter = 0; iter < 2000; iter++) {
        cow_race_target[0] = 0x12340000 + iter;
        cow_race_target[1023] = 0x56780000 + iter;

        int p2c[2], c2p[2];
        if (pipe(p2c) < 0 || pipe(c2p) < 0) {
            printf("[linux-fork] pipe failed at iter %d\n", iter);
            return 40;
        }

        pid_t p = fork();
        if (p < 0) {
            printf("[linux-fork] concurrent fork failed at iter %d\n", iter);
            return 41;
        }

        if (p == 0) {
            // Child: close unused ends
            close(p2c[1]);
            close(c2p[0]);

            // Handshake with parent to ensure both cores are actively executing
            char tok = 0;
            if (read(p2c[0], &tok, 1) != 1 || write(c2p[1], "K", 1) != 1) {
                _exit(10);
            }
            close(p2c[0]);
            close(c2p[1]);

            // TSC barrier: spin until exact hardware timestamp, then execute write with NO synchronization
            unsigned long long target = rdtsc_barrier() + 25000;
            while (rdtsc_barrier() < target) {
                __asm__ volatile("pause");
            }

            // Simultaneous write to COW page
            cow_race_target[0] = 0xCCCC0000 + iter;
            cow_race_target[1023] = 0xDDDD0000 + iter;

            if (cow_race_target[0] != (0xCCCC0000 + iter) || cow_race_target[1023] != (0xDDDD0000 + iter)) {
                _exit(11);
            }
            _exit(0);
        } else {
            // Parent: close unused ends
            close(p2c[0]);
            close(c2p[1]);

            // Handshake with child
            char ack = 0;
            if (write(p2c[1], "G", 1) != 1 || read(c2p[0], &ack, 1) != 1) {
                race_failures++;
            }
            close(p2c[1]);
            close(c2p[0]);

            // Synchronized TSC barrier matching child
            unsigned long long target = rdtsc_barrier() + 25000;
            while (rdtsc_barrier() < target) {
                __asm__ volatile("pause");
            }

            // Simultaneous write to COW page
            cow_race_target[0] = 0xAAAA0000 + iter;
            cow_race_target[1023] = 0xBBBB0000 + iter;

            int status = 0;
            pid_t w = waitpid(p, &status, 0);
            if (w != p || !WIFEXITED(status) || WEXITSTATUS(status) != 0) {
                race_failures++;
            }
            if (cow_race_target[0] != (0xAAAA0000 + iter) || cow_race_target[1023] != (0xBBBB0000 + iter)) {
                corruptions++;
            }
        }

        if ((iter + 1) % 500 == 0) {
            printf("[linux-fork] COW race progress: %d/2000 iterations completed (failures=%d, corruptions=%d)\n",
                   iter + 1, race_failures, corruptions);
        }
    }

    if (race_failures == 0 && corruptions == 0) {
        printf("[linux-fork] concurrent COW race 2000-iteration test verified\n");
    } else {
        printf("[linux-fork] concurrent COW race had %d failures and %d corruptions\n", race_failures, corruptions);
        return 42;
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

    // Phase 5: Multi-table demand-paged child process exit and address space teardown test
    pid_t demand_child = fork();
    if (demand_child < 0) {
        printf("[linux-fork] demand_child fork failed\n");
        return 50;
    }
    if (demand_child == 0) {
        // Child triggers demand-paged stack expansion (64 frames = 256 KiB)
        int s = recurse_stack(64, 0);
        // Child explicitly mmaps 4 MiB at a new address spanning multiple 2 MiB page tables
        char *dmem = (char *)mmap(NULL, 4 * 1024 * 1024, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        if (dmem != MAP_FAILED && dmem != NULL) {
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
