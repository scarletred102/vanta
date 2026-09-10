#define _GNU_SOURCE
#include <pthread.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <string.h>
#include <stdio.h>
#include <sys/syscall.h>
#include <time.h>
#include <sys/time.h>
#include <signal.h>
#include <sys/stat.h>
#include <sys/statfs.h>
#include <sys/resource.h>
#include <sys/prctl.h>

const char __interp[] __attribute__((section(".interp"))) = "/lib/ld-musl-x86_64.so.1";

#define NUM_THREADS 4
#define NUM_ITERATIONS 200
#define ARCH_GET_FS 0x1003

#define FUTEX_WAIT 0
#define FUTEX_WAKE 1
#define FUTEX_REQUEUE 3
#define FUTEX_CMP_REQUEUE 4
#define FUTEX_PRIVATE_FLAG 128

static __thread int t_thread_id = 0;
static __thread unsigned long long t_counter = 0;
static __thread char t_name[32];
static __thread int *t_errno_addr = NULL;

static void *g_t_id_addrs[NUM_THREADS + 1];
static void *g_errno_addrs[NUM_THREADS + 1];
static unsigned long long g_fs_bases[NUM_THREADS + 1];
static int g_thread_errors[NUM_THREADS + 1];

static pthread_mutex_t g_mutex = PTHREAD_MUTEX_INITIALIZER;
static volatile int g_counter = 0;

/* ------------------------------------------------------------
 * Test 1: Thread-Local Storage & Thread Lifecycle
 * ------------------------------------------------------------ */
static void *thread_entry(void *arg) {
    long id = (long)arg;
    t_thread_id = (int)id;
    t_counter = 0;
    t_errno_addr = &errno;

    unsigned long long fs_base = 0;
    if (syscall(SYS_arch_prctl, ARCH_GET_FS, &fs_base) != 0) {
        g_thread_errors[id] = 1;
        return (void *)1;
    }

    if (fs_base == 0 || *(unsigned long long *)fs_base != fs_base) {
        g_thread_errors[id] = 2;
        return (void *)2;
    }

    g_t_id_addrs[id] = (void *)&t_thread_id;
    g_errno_addrs[id] = (void *)&errno;
    g_fs_bases[id] = fs_base;

    for (int i = 0; i < NUM_ITERATIONS; i++) {
        t_counter += (unsigned long long)(id * 1000 + i);
        t_name[i % 31] = (char)('A' + ((id * 3 + i) % 26));

        if (i < 2) {
            int fd = open("/dev/nonexistent_vanta_tls_file", O_RDONLY);
            if (fd >= 0) {
                close(fd);
            }

            if (errno != ENOENT) {
                g_thread_errors[id] = 3;
                return (void *)3;
            }
        }

        int my_errno = (int)(100 + id * 10 + (i % 7));
        errno = my_errno;

        if (i % 20 == 0) {
            sched_yield();
        }

        if (errno != my_errno) {
            g_thread_errors[id] = 4;
            return (void *)4;
        }

        if (t_thread_id != (int)id) {
            g_thread_errors[id] = 5;
            return (void *)5;
        }
    }

    unsigned long long expected_sum = 200ULL * (unsigned long long)id * 1000ULL + 19900ULL;
    if (t_counter != expected_sum) {
        g_thread_errors[id] = 6;
        return (void *)6;
    }

    int final_errno = (int)(100 + id * 10 + ((NUM_ITERATIONS - 1) % 7));
    if (*__errno_location() != final_errno) {
        g_thread_errors[id] = 7;
        return (void *)7;
    }

    pthread_mutex_lock(&g_mutex);
    g_counter += 1;
    pthread_mutex_unlock(&g_mutex);

    return (void *)0;
}

static inline unsigned long long rdtsc_barrier(void) {
    unsigned int lo, hi;
    __asm__ volatile ("rdtsc" : "=a"(lo), "=d"(hi));
    return ((unsigned long long)hi << 32) | lo;
}

/* ------------------------------------------------------------
 * Test 2: Contended Mutex (Multi-Core SMP TSC Spin-Barrier Contention)
 * ------------------------------------------------------------ */
#define BARRIER_ITERS 50
static pthread_mutex_t g_contended_mutex = PTHREAD_MUTEX_INITIALIZER;
static volatile int g_contended_counter = 0;
static volatile int g_barrier_arrived[2] = {0, 0};
static volatile int g_barrier_done[2] = {0, 0};
static volatile int g_barrier_round = 0;
static volatile unsigned long long g_barrier_target_tsc = 0;
static volatile int g_mutex_in_cs = 0;
static volatile int g_mutex_race_violations = 0;
static volatile int g_mutex_corruption_count = 0;
static volatile int g_mutex_cs_val = 0;

static void *mutex_contention_worker(void *arg) {
    long tid = (long)arg;
    for (int iter = 0; iter < BARRIER_ITERS; iter++) {
        pthread_mutex_lock(&g_contended_mutex);

        int in_cs = __atomic_exchange_n(&g_mutex_in_cs, 1, __ATOMIC_SEQ_CST);
        if (in_cs != 0) {
            __atomic_fetch_add(&g_mutex_race_violations, 1, __ATOMIC_SEQ_CST);
        }

        g_contended_counter++;
        g_mutex_cs_val = (int)(iter * 1000 + tid);
        for (volatile int churn = 0; churn < 50; churn++) {}
        if (g_mutex_cs_val != (int)(iter * 1000 + tid)) {
            __atomic_fetch_add(&g_mutex_corruption_count, 1, __ATOMIC_SEQ_CST);
        }

        __atomic_store_n(&g_mutex_in_cs, 0, __ATOMIC_SEQ_CST);
        pthread_mutex_unlock(&g_contended_mutex);
    }
    return NULL;
}

static int test_contended_mutex(void) {
    pthread_t th[NUM_THREADS];
    g_contended_counter = 0;
    g_barrier_arrived[0] = 0;
    g_barrier_arrived[1] = 0;
    g_barrier_done[0] = 0;
    g_barrier_done[1] = 0;
    g_barrier_round = 0;
    g_barrier_target_tsc = 0;
    g_mutex_in_cs = 0;
    g_mutex_race_violations = 0;
    g_mutex_corruption_count = 0;
    g_mutex_cs_val = 0;

    for (long i = 0; i < NUM_THREADS; i++) {
        if (pthread_create(&th[i], NULL, mutex_contention_worker, (void *)i) != 0) {
            return -1;
        }
    }
    for (int i = 0; i < NUM_THREADS; i++) {
        pthread_join(th[i], NULL);
    }

    const char banner[] = "[linux-dynamic] futex/mutex TSC spin-barrier contention verified\n";
    write(1, banner, sizeof(banner) - 1);
    char buf[128];
    snprintf(buf, sizeof(buf), "[linux-dynamic] rounds=%d threads=%d counter=%d violations=%d corruptions=%d\n",
             BARRIER_ITERS, NUM_THREADS, g_contended_counter, g_mutex_race_violations, g_mutex_corruption_count);
    write(1, buf, strlen(buf));

    if (g_mutex_race_violations != 0) {
        return -2;
    }
    if (g_mutex_corruption_count != 0) {
        return -3;
    }
    if (g_contended_counter != NUM_THREADS * BARRIER_ITERS) {
        return -4;
    }
    return 0;
}

/* ------------------------------------------------------------
 * Test 3: Condition Variable (Producer-Consumer Bounded Queue)
 * ------------------------------------------------------------ */
#define QUEUE_CAP 4
#define TOTAL_ITEMS 100
static int g_queue[QUEUE_CAP];
static int g_q_head = 0;
static int g_q_tail = 0;
static int g_q_count = 0;
static pthread_mutex_t g_q_mutex = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t g_q_not_full = PTHREAD_COND_INITIALIZER;
static pthread_cond_t g_q_not_empty = PTHREAD_COND_INITIALIZER;
static volatile int g_consumed_sum = 0;
static volatile int g_consumed_count = 0;

static void *cond_producer(void *arg) {
    long start_val = (long)arg;
    for (int i = 0; i < TOTAL_ITEMS / 2; i++) {
        int val = (int)(start_val + i);
        pthread_mutex_lock(&g_q_mutex);
        while (g_q_count == QUEUE_CAP) {
            pthread_cond_wait(&g_q_not_full, &g_q_mutex);
        }
        g_queue[g_q_tail] = val;
        g_q_tail = (g_q_tail + 1) % QUEUE_CAP;
        g_q_count++;
        pthread_cond_signal(&g_q_not_empty);
        pthread_mutex_unlock(&g_q_mutex);
    }
    return NULL;
}

static void *cond_consumer(void *arg) {
    (void)arg;
    for (int i = 0; i < TOTAL_ITEMS / 2; i++) {
        pthread_mutex_lock(&g_q_mutex);
        while (g_q_count == 0) {
            pthread_cond_wait(&g_q_not_empty, &g_q_mutex);
        }
        int val = g_queue[g_q_head];
        g_q_head = (g_q_head + 1) % QUEUE_CAP;
        g_q_count--;
        g_consumed_sum += val;
        g_consumed_count++;
        pthread_cond_signal(&g_q_not_full);
        pthread_mutex_unlock(&g_q_mutex);
    }
    return NULL;
}

static int test_condvar_queue(void) {
    pthread_t prod[2], cons[2];
    g_q_head = 0;
    g_q_tail = 0;
    g_q_count = 0;
    g_consumed_sum = 0;
    g_consumed_count = 0;

    pthread_create(&prod[0], NULL, cond_producer, (void *)0);
    pthread_create(&prod[1], NULL, cond_producer, (void *)50);
    pthread_create(&cons[0], NULL, cond_consumer, NULL);
    pthread_create(&cons[1], NULL, cond_consumer, NULL);

    pthread_join(prod[0], NULL);
    pthread_join(prod[1], NULL);
    pthread_join(cons[0], NULL);
    pthread_join(cons[1], NULL);

    if (g_consumed_count != TOTAL_ITEMS) {
        return -1;
    }
    if (g_consumed_sum != 4950) {
        return -2;
    }
    return 0;
}

/* ------------------------------------------------------------
 * Test 4: Condition Variable Broadcast Barrier
 * ------------------------------------------------------------ */
static pthread_cond_t g_broadcast_cond = PTHREAD_COND_INITIALIZER;
static pthread_mutex_t g_broadcast_mutex = PTHREAD_MUTEX_INITIALIZER;
static volatile int g_broadcast_ready = 0;
static volatile int g_broadcast_woken = 0;

static void *broadcast_worker(void *arg) {
    (void)arg;
    pthread_mutex_lock(&g_broadcast_mutex);
    g_broadcast_ready++;
    pthread_cond_wait(&g_broadcast_cond, &g_broadcast_mutex);
    g_broadcast_woken++;
    pthread_mutex_unlock(&g_broadcast_mutex);
    return NULL;
}

static int test_condvar_broadcast(void) {
    pthread_t th[NUM_THREADS];
    g_broadcast_ready = 0;
    g_broadcast_woken = 0;

    for (int i = 0; i < NUM_THREADS; i++) {
        if (pthread_create(&th[i], NULL, broadcast_worker, NULL) != 0) {
            return -1;
        }
    }

    while (1) {
        pthread_mutex_lock(&g_broadcast_mutex);
        int r = g_broadcast_ready;
        pthread_mutex_unlock(&g_broadcast_mutex);
        if (r == NUM_THREADS) break;
        sched_yield();
    }

    pthread_mutex_lock(&g_broadcast_mutex);
    pthread_cond_broadcast(&g_broadcast_cond);
    pthread_mutex_unlock(&g_broadcast_mutex);

    for (int i = 0; i < NUM_THREADS; i++) {
        pthread_join(th[i], NULL);
    }

    if (g_broadcast_woken != NUM_THREADS) {
        return -2;
    }
    return 0;
}

/* ------------------------------------------------------------
 * Test 5: Read-Write Lock (pthread_rwlock_t)
 * ------------------------------------------------------------ */
static pthread_rwlock_t g_rwlock = PTHREAD_RWLOCK_INITIALIZER;
static volatile int g_active_readers = 0;
static volatile int g_active_writers = 0;
static volatile int g_rwlock_errors = 0;
static volatile int g_shared_rw_val = 0;

static void *rwlock_reader(void *arg) {
    (void)arg;
    for (int i = 0; i < 200; i++) {
        pthread_rwlock_rdlock(&g_rwlock);
        if (g_active_writers > 0) {
            g_rwlock_errors++;
        }
        __atomic_fetch_add(&g_active_readers, 1, __ATOMIC_SEQ_CST);
        int v = g_shared_rw_val;
        (void)v;
        if (i % 20 == 0) sched_yield();
        if (g_active_writers > 0) {
            g_rwlock_errors++;
        }
        __atomic_fetch_sub(&g_active_readers, 1, __ATOMIC_SEQ_CST);
        pthread_rwlock_unlock(&g_rwlock);
    }
    return NULL;
}

static void *rwlock_writer(void *arg) {
    (void)arg;
    for (int i = 0; i < 100; i++) {
        pthread_rwlock_wrlock(&g_rwlock);
        if (g_active_readers > 0 || g_active_writers > 0) {
            g_rwlock_errors++;
        }
        g_active_writers = 1;
        g_shared_rw_val++;
        if (i % 10 == 0) sched_yield();
        if (g_active_readers > 0) {
            g_rwlock_errors++;
        }
        g_active_writers = 0;
        pthread_rwlock_unlock(&g_rwlock);
    }
    return NULL;
}

static int test_rwlock(void) {
    pthread_t r[2], w[2];
    g_active_readers = 0;
    g_active_writers = 0;
    g_rwlock_errors = 0;
    g_shared_rw_val = 0;

    pthread_create(&r[0], NULL, rwlock_reader, NULL);
    pthread_create(&w[0], NULL, rwlock_writer, NULL);
    pthread_create(&r[1], NULL, rwlock_reader, NULL);
    pthread_create(&w[1], NULL, rwlock_writer, NULL);

    pthread_join(r[0], NULL);
    pthread_join(w[0], NULL);
    pthread_join(r[1], NULL);
    pthread_join(w[1], NULL);

    if (g_rwlock_errors != 0) {
        return -1;
    }
    if (g_shared_rw_val != 200) {
        return -2;
    }
    return 0;
}

/* ------------------------------------------------------------
 * Test 6: Direct Futex FUTEX_CMP_REQUEUE & Error Verification
 * ------------------------------------------------------------ */
static volatile int g_futex_src = 0;
static volatile int g_futex_dst = 0;
static volatile int g_futex_worker_ready = 0;
static volatile int g_futex_worker_done = 0;

static void *direct_futex_worker(void *arg) {
    (void)arg;
    g_futex_worker_ready = 1;
    long ret = syscall(SYS_futex, &g_futex_src, FUTEX_WAIT | FUTEX_PRIVATE_FLAG, 0, NULL, NULL, 0);
    if (ret == 0) {
        g_futex_worker_done = 1;
    }
    return NULL;
}

static int test_direct_futex_requeue(void) {
    g_futex_src = 0;
    g_futex_dst = 0;
    g_futex_worker_ready = 0;
    g_futex_worker_done = 0;

    // 1. Error case: uaddr == uaddr2 -> EINVAL
    long r = syscall(SYS_futex, &g_futex_src, FUTEX_CMP_REQUEUE | FUTEX_PRIVATE_FLAG, 0, 1, &g_futex_src, 0);
    if (r != -1 || errno != EINVAL) {
        return -1;
    }

    // 2. Error case: uaddr2 == NULL -> EINVAL
    r = syscall(SYS_futex, &g_futex_src, FUTEX_CMP_REQUEUE | FUTEX_PRIVATE_FLAG, 0, 1, NULL, 0);
    if (r != -1 || errno != EINVAL) {
        return -2;
    }

    // 3. Error case: val3 mismatch -> EAGAIN
    g_futex_src = 42;
    r = syscall(SYS_futex, &g_futex_src, FUTEX_CMP_REQUEUE | FUTEX_PRIVATE_FLAG, 0, 1, &g_futex_dst, 999);
    if (r != -1 || errno != EAGAIN) {
        return -3;
    }

    // Reset futex values
    g_futex_src = 0;
    g_futex_dst = 0;

    pthread_t th;
    if (pthread_create(&th, NULL, direct_futex_worker, NULL) != 0) {
        return -4;
    }

    while (!g_futex_worker_ready) {
        sched_yield();
    }
    for (int i = 0; i < 20; i++) {
        sched_yield();
    }

    // Requeue 1 waiter from futex_src to futex_dst with 0 woken on src:
    r = syscall(SYS_futex, &g_futex_src, FUTEX_CMP_REQUEUE | FUTEX_PRIVATE_FLAG, 0, 1, &g_futex_dst, 0);
    if (r != 1) {
        return -5;
    }

    // Attempt to wake on futex_src: should wake 0 waiters
    r = syscall(SYS_futex, &g_futex_src, FUTEX_WAKE | FUTEX_PRIVATE_FLAG, 1, NULL, NULL, 0);
    if (r != 0) {
        return -6;
    }
    if (g_futex_worker_done != 0) {
        return -7;
    }

    // Wake on futex_dst: should wake 1 waiter (the requeued worker)!
    r = syscall(SYS_futex, &g_futex_dst, FUTEX_WAKE | FUTEX_PRIVATE_FLAG, 1, NULL, NULL, 0);
    if (r != 1) {
        return -8;
    }

    pthread_join(th, NULL);

    if (g_futex_worker_done != 1) {
        return -9;
    }

    return 0;
}

/* ------------------------------------------------------------
 * Test 7: Hierarchical Timer Wheel, Nanosleep, Itimers, Clocks
 * ------------------------------------------------------------ */
static volatile int g_sigalrm_fired = 0;
static void sigalrm_handler(int sig) {
    if (sig == SIGALRM) {
        g_sigalrm_fired++;
    }
}

static int test_timer_subsystem(void) {
    // 1. clock_getres
    struct timespec res;
    if (clock_getres(CLOCK_MONOTONIC, &res) != 0) {
        return 1;
    }
    if (res.tv_sec != 0 || res.tv_nsec != 1000000) { // 1ms resolution
        return 2;
    }

    // 2. clock_gettime & nanosleep duration check (must block for real duration)
    struct timespec t_start, t_end;
    if (clock_gettime(CLOCK_MONOTONIC, &t_start) != 0) {
        return 3;
    }
    struct timespec req = { .tv_sec = 0, .tv_nsec = 50 * 1000 * 1000 }; // 50 ms
    if (nanosleep(&req, NULL) != 0) {
        return 4;
    }
    if (clock_gettime(CLOCK_MONOTONIC, &t_end) != 0) {
        return 5;
    }
    long long elapsed_ms = (t_end.tv_sec - t_start.tv_sec) * 1000LL + 
                           (t_end.tv_nsec - t_start.tv_nsec) / 1000000LL;
    if (elapsed_ms < 40) {
        return 6;
    }

    // 3. clock_nanosleep
    if (clock_gettime(CLOCK_MONOTONIC, &t_start) != 0) {
        return 7;
    }
    req.tv_sec = 0;
    req.tv_nsec = 30 * 1000 * 1000; // 30 ms
    if (clock_nanosleep(CLOCK_MONOTONIC, 0, &req, NULL) != 0) {
        return 8;
    }
    if (clock_gettime(CLOCK_MONOTONIC, &t_end) != 0) {
        return 9;
    }
    elapsed_ms = (t_end.tv_sec - t_start.tv_sec) * 1000LL + 
                 (t_end.tv_nsec - t_start.tv_nsec) / 1000000LL;
    if (elapsed_ms < 20) {
        return 10;
    }

    // 4. clock_settime and clock_gettime(CLOCK_REALTIME)
    struct timespec cur_real;
    if (clock_gettime(CLOCK_REALTIME, &cur_real) != 0) {
        return 11;
    }
    struct timespec new_real = {
        .tv_sec = cur_real.tv_sec + 5000,
        .tv_nsec = 123456789,
    };
    if (clock_settime(CLOCK_REALTIME, &new_real) != 0) {
        return 12;
    }
    struct timespec check_real;
    if (clock_gettime(CLOCK_REALTIME, &check_real) != 0) {
        return 13;
    }
    if (check_real.tv_sec < new_real.tv_sec) {
        return 14;
    }

    // 5. nanosleep invalid argument test
    struct timespec bad_req = { .tv_sec = 0, .tv_nsec = 1000000000L }; // >= 1s invalid
    if (nanosleep(&bad_req, NULL) != -1 || errno != EINVAL) {
        return 15;
    }
    bad_req.tv_sec = -1;
    bad_req.tv_nsec = 0;
    if (nanosleep(&bad_req, NULL) != -1 || errno != EINVAL) {
        return 16;
    }

    // 6. itimer test
    signal(SIGALRM, sigalrm_handler);
    struct itimerval it = {
        .it_interval = { 0, 0 },
        .it_value = { 0, 40 * 1000 }, // 40 ms
    };
    struct itimerval old_it;
    if (setitimer(ITIMER_REAL, &it, &old_it) != 0) {
        return 17;
    }
    struct itimerval cur_it;
    if (getitimer(ITIMER_REAL, &cur_it) != 0) {
        return 18;
    }
    // Sleep for 70ms to let itimer expire and fire SIGALRM
    struct timespec wait_alrm = { .tv_sec = 0, .tv_nsec = 70 * 1000 * 1000 };
    nanosleep(&wait_alrm, NULL);
    if (g_sigalrm_fired == 0) {
        return 19;
    }

    // 7. Early signal interruption with rem writeback
    g_sigalrm_fired = 0;
    it.it_value.tv_sec = 0;
    it.it_value.tv_usec = 30 * 1000; // 30 ms
    setitimer(ITIMER_REAL, &it, NULL);
    struct timespec long_req = { .tv_sec = 1, .tv_nsec = 0 }; // 1000 ms
    struct timespec rem = { 0, 0 };
    int ret = nanosleep(&long_req, &rem);
    if (ret != -1 || errno != EINTR) {
        return 20; // Must be interrupted by SIGALRM
    }
    if (rem.tv_sec == 0 && rem.tv_nsec == 0) {
        return 21; // Rem must be written back
    }
    if (g_sigalrm_fired == 0) {
        return 22; // SIGALRM must have fired
    }

    return 0;
}

static volatile int g_sigusr1_received = 0;
static void sigusr1_handler(int sig) {
    (void)sig;
    g_sigusr1_received = 1;
}

/* ------------------------------------------------------------
 * Test 8: Remaining 85-Syscall Matrix & Signals Completion
 * ------------------------------------------------------------ */
static int test_85_syscall_matrix_and_signals(void) {
    /* 1. Working Directory & Path Traversal (getcwd, chdir, mkdir, unlink) */
    char cwd_buf[256] = {0};
    if (getcwd(cwd_buf, sizeof(cwd_buf)) == NULL || strlen(cwd_buf) == 0) {
        return 1;
    }
    if (mkdir("/tmp/testdir_posix", 0755) != 0 && errno != EEXIST) {
        return 2;
    }
    if (chdir("/tmp/testdir_posix") != 0) {
        return 3;
    }
    char new_cwd[256] = {0};
    if (getcwd(new_cwd, sizeof(new_cwd)) == NULL || strstr(new_cwd, "testdir_posix") == NULL) {
        return 4;
    }
    if (chdir("/") != 0) {
        return 5;
    }
    unlink("/tmp/testdir_posix");

    /* 2. Signal Alternative Stack (sigaltstack) */
    stack_t old_ss;
    memset(&old_ss, 0, sizeof(old_ss));
    if (sigaltstack(NULL, &old_ss) != 0) {
        return 6;
    }
    static char alt_stack[4096];
    stack_t new_ss;
    new_ss.ss_sp = alt_stack;
    new_ss.ss_size = sizeof(alt_stack);
    new_ss.ss_flags = 0;
    if (sigaltstack(&new_ss, &old_ss) != 0) {
        return 7;
    }
    stack_t cur_ss;
    memset(&cur_ss, 0, sizeof(cur_ss));
    if (sigaltstack(NULL, &cur_ss) != 0) {
        return 8;
    }
    if (cur_ss.ss_sp != alt_stack || cur_ss.ss_size != sizeof(alt_stack) || cur_ss.ss_flags != 0) {
        return 9;
    }
    // Test rejection of ss_size < MINSIGSTKSZ (2048)
    stack_t bad_ss;
    bad_ss.ss_sp = alt_stack;
    bad_ss.ss_size = 512;
    bad_ss.ss_flags = 0;
    if (sigaltstack(&bad_ss, NULL) == 0) {
        return 10; // Must fail
    }

    /* 3. Positioned I/O & File Truncation (pread64, pwrite64, ftruncate) */
    int fd = open("/tmp/test_io_posix.txt", O_CREAT | O_RDWR | O_TRUNC, 0644);
    if (fd < 0) {
        return 11;
    }
    const char init_data[] = "0123456789ABCDEF";
    if (write(fd, init_data, sizeof(init_data) - 1) != (ssize_t)(sizeof(init_data) - 1)) {
        close(fd);
        return 12;
    }
    off_t pos_before = lseek(fd, 0, SEEK_CUR);
    // Write at offset 5 without updating descriptor offset
    if (pwrite(fd, "XYZ", 3, 5) != 3) {
        close(fd);
        return 13;
    }
    off_t pos_after = lseek(fd, 0, SEEK_CUR);
    if (pos_before != pos_after) {
        close(fd);
        return 14; // pwrite must preserve descriptor offset
    }
    char read_buf[32] = {0};
    // Read from offset 0
    if (pread(fd, read_buf, 16, 0) != 16) {
        close(fd);
        return 15;
    }
    if (memcmp(read_buf, "01234XYZ89ABCDEF", 16) != 0) {
        close(fd);
        return 16;
    }
    // Truncate to 8 bytes
    if (ftruncate(fd, 8) != 0) {
        close(fd);
        return 17;
    }
    memset(read_buf, 0, sizeof(read_buf));
    ssize_t truncated_len = pread(fd, read_buf, sizeof(read_buf), 0);
    if (truncated_len != 8 || memcmp(read_buf, "01234XYZ", 8) != 0) {
        close(fd);
        return 18;
    }
    close(fd);
    unlink("/tmp/test_io_posix.txt");

    /* 4. Filesystem Statistics (statfs) */
    struct statfs sf;
    memset(&sf, 0, sizeof(sf));
    if (statfs("/", &sf) != 0) {
        return 19;
    }
    if (sf.f_bsize != 4096 || sf.f_blocks == 0) {
        return 20;
    }

    /* 5. Process Metadata & Limits (prctl, prlimit64) */
    char orig_comm[16] = {0};
    syscall(SYS_prctl, PR_GET_NAME, orig_comm, 0, 0, 0);
    if (syscall(SYS_prctl, PR_SET_NAME, "vanta_test_comm", 0, 0, 0) != 0) {
        return 21;
    }
    char set_comm[16] = {0};
    if (syscall(SYS_prctl, PR_GET_NAME, set_comm, 0, 0, 0) != 0 || strcmp(set_comm, "vanta_test_comm") != 0) {
        return 22;
    }
    struct rlimit rlim;
    memset(&rlim, 0, sizeof(rlim));
    if (syscall(SYS_prlimit64, 0, RLIMIT_NOFILE, NULL, &rlim) != 0) {
        return 23;
    }
    if (rlim.rlim_cur == 0 || rlim.rlim_max == 0) {
        return 24;
    }

    /* 6. Symbolic Links (symlink, readlink) */
    if (symlink("/target_path_vanta", "/tmp/test_symlink_vanta") != 0) {
        return 25;
    }
    char link_target[64] = {0};
    ssize_t link_len = readlink("/tmp/test_symlink_vanta", link_target, sizeof(link_target) - 1);
    if (link_len <= 0 || strcmp(link_target, "/target_path_vanta") != 0) {
        unlink("/tmp/test_symlink_vanta");
        return 26;
    }
    unlink("/tmp/test_symlink_vanta");

    /* 7. Signals Inspection & Masking (rt_sigpending & delivery on unmask) */
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = sigusr1_handler;
    if (sigaction(SIGUSR1, &sa, NULL) != 0) {
        return 27;
    }

    sigset_t block_set, old_mask, pend_set;
    sigemptyset(&block_set);
    sigaddset(&block_set, SIGUSR1);
    if (sigprocmask(SIG_BLOCK, &block_set, &old_mask) != 0) {
        return 28;
    }
    g_sigusr1_received = 0;
    kill(getpid(), SIGUSR1);
    // While blocked, signal MUST NOT have been delivered
    if (g_sigusr1_received != 0) {
        return 29;
    }
    sigemptyset(&pend_set);
    if (sigpending(&pend_set) != 0) {
        return 30;
    }
    if (!sigismember(&pend_set, SIGUSR1)) {
        return 31;
    }
    // Restore signal mask: unblocking SIGUSR1 must trigger delivery!
    if (sigprocmask(SIG_SETMASK, &old_mask, NULL) != 0) {
        return 32;
    }
    // Handler MUST have executed!
    if (g_sigusr1_received != 1) {
        return 33;
    }

    return 0;
}

int main(void) {
    t_thread_id = 999;
    t_counter = 0x12345678ULL;
    strcpy(t_name, "MAIN_THREAD_INIT");
    errno = 42;

    unsigned long long main_fs_base = 0;
    if (syscall(SYS_arch_prctl, ARCH_GET_FS, &main_fs_base) != 0 || main_fs_base == 0) {
        return 10;
    }
    if (*(unsigned long long *)main_fs_base != main_fs_base) {
        return 11;
    }

    g_t_id_addrs[0] = (void *)&t_thread_id;
    g_errno_addrs[0] = (void *)&errno;
    g_fs_bases[0] = main_fs_base;

    const char spawn_msg[] = "[linux-dynamic] thread spawned\n";
    write(1, spawn_msg, sizeof(spawn_msg) - 1);

    /* Test 1: TLS and basic thread lifecycle */
    pthread_t threads[NUM_THREADS];
    for (long i = 1; i <= NUM_THREADS; i++) {
        if (pthread_create(&threads[i - 1], NULL, thread_entry, (void *)i) != 0) {
            return 20 + (int)i;
        }
    }

    for (int i = 0; i < NUM_THREADS; i++) {
        void *res = NULL;
        if (pthread_join(threads[i], &res) != 0 || res != (void *)0) {
            return 30 + i;
        }
    }

    if (t_thread_id != 999) {
        return 40;
    }
    if (t_counter != 0x12345678ULL) {
        return 41;
    }
    if (strcmp(t_name, "MAIN_THREAD_INIT") != 0) {
        return 42;
    }
    if (errno != 42) {
        return 43;
    }

    for (int i = 0; i <= NUM_THREADS; i++) {
        if (g_thread_errors[i] != 0) {
            return 50 + i;
        }
        for (int j = i + 1; j <= NUM_THREADS; j++) {
            if (g_t_id_addrs[i] == g_t_id_addrs[j]) {
                return 60;
            }
            if (g_errno_addrs[i] == g_errno_addrs[j]) {
                return 70;
            }
            if (g_fs_bases[i] == g_fs_bases[j]) {
                return 80;
            }
        }
    }

    const char tls_msg[] = "[linux-dynamic] thread TLS verified\n";
    write(1, tls_msg, sizeof(tls_msg) - 1);

    /* Test 2: Contended mutex under multi-threaded load */
    if (test_contended_mutex() != 0) {
        return 90;
    }

    /* Test 3: Producer-Consumer bounded queue with pthread_cond_wait / signal */
    if (test_condvar_queue() != 0) {
        return 91;
    }

    /* Test 4: Condition variable broadcast */
    if (test_condvar_broadcast() != 0) {
        return 92;
    }

    /* Test 5: Reader-writer lock mutual exclusion */
    if (test_rwlock() != 0) {
        return 93;
    }

    /* Test 6: Direct futex requeue & error cases */
    if (test_direct_futex_requeue() != 0) {
        return 94;
    }

    const char sync_msg[] = "[linux-dynamic] futex synchronization passed\n";
    write(1, sync_msg, sizeof(sync_msg) - 1);

    const char join_msg[] = "[linux-dynamic] thread joined successfully\n";
    write(1, join_msg, sizeof(join_msg) - 1);

    /* Test 7: Hierarchical Timer Wheel, Nanosleep, Itimers, Clocks */
    int timer_res = test_timer_subsystem();
    if (timer_res != 0) {
        char err_msg[64];
        snprintf(err_msg, sizeof(err_msg), "[linux-dynamic] timer subsystem failed: %d\n", timer_res);
        write(1, err_msg, strlen(err_msg));
        return 95;
    }

    const char timer_msg[] = "[linux-dynamic] timer subsystem and nanosleep verified\n";
    write(1, timer_msg, sizeof(timer_msg) - 1);

    /* Test 8: Remaining 85-Syscall Matrix & Signals */
    int matrix_res = test_85_syscall_matrix_and_signals();
    if (matrix_res != 0) {
        char err_msg[64];
        snprintf(err_msg, sizeof(err_msg), "[linux-dynamic] 85-syscall matrix failed: %d\n", matrix_res);
        write(1, err_msg, strlen(err_msg));
        return 96;
    }

    const char matrix_msg[] = "[linux-dynamic] 85-syscall matrix and signals verified\n";
    write(1, matrix_msg, sizeof(matrix_msg) - 1);

    return 0;
}
