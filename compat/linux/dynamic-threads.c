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

    const char sync_msg[] = "[linux-dynamic] futex synchronization passed\n";
    write(1, sync_msg, sizeof(sync_msg) - 1);

    const char join_msg[] = "[linux-dynamic] thread joined successfully\n";
    write(1, join_msg, sizeof(join_msg) - 1);

    return 0;
}
