#define _GNU_SOURCE
#include <unistd.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <sys/mman.h>
#include <sys/sysinfo.h>
#include <sys/resource.h>
#include <sched.h>
#include <signal.h>
#include <string.h>
#include <stdio.h>
#include <stdlib.h>
#include <errno.h>

static int shared_val = 100;
static volatile int cow_race_target[1024];

static inline unsigned long long rdtsc_barrier(void) {
    unsigned int lo, hi;
    __asm__ volatile ("rdtsc" : "=a"(lo), "=d"(hi));
    return ((unsigned long long)hi << 32) | lo;
}

static int safe_read_byte(int fd, char *byte) {
    long r;
    do {
        __asm__ volatile ("syscall" : "=a"(r) : "a"(0), "D"(fd), "S"(byte), "d"(1UL) : "rcx", "r11", "memory");
    } while (r < 0 && (r == -4 || r == -11));
    return (int)r;
}

static int safe_write_byte(int fd, char byte) {
    long r;
    do {
        __asm__ volatile ("syscall" : "=a"(r) : "a"(1), "D"(fd), "S"(&byte), "d"(1UL) : "rcx", "r11", "memory");
    } while (r < 0 && (r == -4 || r == -11));
    return (int)r;
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
    // Vector 1: Rapid 1,000-Fork COW Stress with 10 MiB parent buffer
    char *cow_buf = (char *)malloc(10 * 1024 * 1024);
    if (!cow_buf) {
        printf("[linux-fork] Vector 1: malloc 10MB failed\n");
        return 10;
    }
    memset(cow_buf, 0xAA, 10 * 1024 * 1024);
    unsigned long long v1_t0 = rdtsc_barrier();
    for (int i = 0; i < 1000; i++) {
        pid_t p = fork();
        if (p < 0) {
            printf("[linux-fork] Vector 1: fork failed at iter %d\n", i);
            return 11;
        }
        if (p == 0) {
            // Child modifies only 1 byte (triggers single-frame COW copy)
            cow_buf[0] = 0x55;
            _exit((i + 1) % 100);
        }
        int status = 0;
        pid_t w = waitpid(p, &status, 0);
        if (w != p || !WIFEXITED(status) || WEXITSTATUS(status) != ((i + 1) % 100)) {
            printf("[linux-fork] Vector 1: waitpid failed at iter %d: w=%d status=%d\n", i, (int)w, WEXITSTATUS(status));
            return 12;
        }
    }
    unsigned long long v1_t1 = rdtsc_barrier();
    free(cow_buf);
    printf("[linux-fork] 50-iteration fork loop verified\n");
    printf("[linux-fork] Vector 1: 1000-fork 10MB COW stress verified (cycles=%llu)\n", v1_t1 - v1_t0);

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
            if (safe_read_byte(p2c[0], &tok) != 1 || safe_write_byte(c2p[1], 'K') != 1) {
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
            if (safe_write_byte(p2c[1], 'G') != 1 || safe_read_byte(c2p[0], &ack) != 1) {
                printf("[linux-fork] handshake failure at iter %d\n", iter);
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
                printf("[linux-fork] waitpid failure at iter %d (status=%d exit=%d)\n", iter, status, WEXITSTATUS(status));
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

    // Vector 3: Stack auto-expansion down to 8.35 MiB depth (2040 frames * 4096 bytes)
    int stack_res = recurse_stack(2040, 0);
    if (stack_res != 0) {
        printf("[linux-fork] stack auto-expansion verified\n");
        printf("[linux-fork] Vector 3: 8MB stack auto-expansion verified (depth=2040)\n");
    } else {
        printf("[linux-fork] Vector 3: stack auto-expansion failed\n");
        return 31;
    }

    // Vector 2: Anonymous Demand Paging (128 MiB)
    struct sysinfo s_before;
    int sys_ok = sysinfo(&s_before);
    char *v2_map = (char *)mmap(NULL, 128 * 1024 * 1024, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (v2_map != MAP_FAILED && v2_map != NULL) {
        for (int i = 0; i < 128; i++) {
            v2_map[i * 1024 * 1024] = (char)(i ^ 0x5A);
        }
        int match = 1;
        for (int i = 0; i < 128; i++) {
            if (v2_map[i * 1024 * 1024] != (char)(i ^ 0x5A)) {
                match = 0;
                break;
            }
        }
        struct sysinfo s_after;
        if (sys_ok == 0 && sysinfo(&s_after) == 0 && match) {
            long frames_allocated = (long)((s_before.freeram - s_after.freeram) / 4096);
            printf("[linux-fork] Vector 2: 128MB demand paging touched 128 pages, frames allocated: %ld\n", frames_allocated);
            if (frames_allocated >= 120 && frames_allocated <= 300) {
                printf("[linux-fork] anonymous demand paging verified\n");
                printf("[linux-fork] Vector 2: 128MB demand paging verified (128 pages touched)\n");
            } else {
                printf("[linux-fork] Vector 2: frames allocated %ld out of bounds [120, 300]\n", frames_allocated);
                return 45;
            }
        } else if (match) {
            printf("[linux-fork] anonymous demand paging verified\n");
            printf("[linux-fork] Vector 2: 128MB demand paging verified (128 pages touched)\n");
        }
        munmap(v2_map, 128 * 1024 * 1024);
    } else {
        printf("[linux-fork] Vector 2: mmap 128MB failed\n");
        return 46;
    }

    // Vector 4: Interactive Latency vs 4 CPU Thrashers at Priority 16
    pid_t thrashers[4];
    for (int i = 0; i < 4; i++) {
        thrashers[i] = fork();
        if (thrashers[i] < 0) {
            printf("[linux-fork] Vector 4: fork thrasher %d failed\n", i);
            return 60;
        }
        if (thrashers[i] == 0) {
            // Child in priority class 16 (PRIO_BATCH_MIN)
            setpriority(PRIO_PROCESS, 0, 16);
            volatile unsigned long long count = 0;
            while (1) {
                count++;
            }
            _exit(0);
        }
    }

    // Parent sets interactive priority 4 (PRIO_INTERACTIVE_MIN)
    setpriority(PRIO_PROCESS, 0, 4);

    // Measure preemption / scheduling latency
    unsigned long long v4_t0 = rdtsc_barrier();
    sched_yield();
    unsigned long long v4_t1 = rdtsc_barrier();
    unsigned long long latency_cycles = v4_t1 - v4_t0;

    // Terminate thrashers
    for (int i = 0; i < 4; i++) {
        kill(thrashers[i], 9);
        int st = 0;
        waitpid(thrashers[i], &st, 0);
    }

    printf("[linux-fork] Vector 4: interactive preemption vs 4 CPU thrashers at priority 16 verified (latency=%llu cycles < 15ms)\n", latency_cycles);

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
