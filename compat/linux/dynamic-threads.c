#define _GNU_SOURCE
#include <pthread.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <string.h>
#include <stdio.h>
#include <sys/syscall.h>

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

        int fd = open("/dev/nonexistent_vanta_tls_file", O_RDONLY);
        if (fd >= 0) {
            close(fd);
        }

        if (errno != ENOENT) {
            g_thread_errors[id] = 3;
            return (void *)3;
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

/* ------------------------------------------------------------
 * Test 2: Contended Mutex (Multi-Core SMP Contention)
 * ------------------------------------------------------------ */
#define MUTEX_ITERS 1000
static pthread_mutex_t g_contended_mutex = PTHREAD_MUTEX_INITIALIZER;
static volatile int g_contended_counter = 0;

static void *mutex_contention_worker(void *arg) {
    (void)arg;
    for (int i = 0; i < MUTEX_ITERS; i++) {
        pthread_mutex_lock(&g_contended_mutex);
        g_contended_counter++;
        pthread_mutex_unlock(&g_contended_mutex);
        if (i % 50 == 0) {
            sched_yield();
        }
    }
    return NULL;
}

static int test_contended_mutex(void) {
    pthread_t th[NUM_THREADS];
    g_contended_counter = 0;
    for (int i = 0; i < NUM_THREADS; i++) {
        if (pthread_create(&th[i], NULL, mutex_contention_worker, NULL) != 0) {
            return -1;
        }
    }
    for (int i = 0; i < NUM_THREADS; i++) {
        pthread_join(th[i], NULL);
    }
    if (g_contended_counter != NUM_THREADS * MUTEX_ITERS) {
        return -2;
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

    return 0;
}
