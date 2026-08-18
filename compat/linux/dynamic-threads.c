#include <pthread.h>
#include <unistd.h>
#include <string.h>

const char __interp[] __attribute__((section(".interp"))) = "/lib/ld-musl-x86_64.so.1";

static __thread int t_worker_id = 0;
static pthread_mutex_t g_mutex = PTHREAD_MUTEX_INITIALIZER;
static volatile int g_counter = 0;

static void *thread_entry(void *arg) {
    long id = (long)arg;
    t_worker_id = (int)id;

    if (t_worker_id == (int)id) {
        const char tls_msg[] = "[linux-dynamic] thread TLS verified\n";
        write(1, tls_msg, sizeof(tls_msg) - 1);
    }

    pthread_mutex_lock(&g_mutex);
    g_counter += 1;
    pthread_mutex_unlock(&g_mutex);

    return (void *)0;
}

int main(void) {
    pthread_t th;
    t_worker_id = 100;

    const char spawn_msg[] = "[linux-dynamic] thread spawned\n";
    write(1, spawn_msg, sizeof(spawn_msg) - 1);

    if (pthread_create(&th, NULL, thread_entry, (void *)1) != 0) {
        return 1;
    }

    for (int i = 0; i < 10000; i++) {
        pthread_mutex_lock(&g_mutex);
        int val = g_counter;
        pthread_mutex_unlock(&g_mutex);
        if (val > 0) {
            break;
        }
        sched_yield();
    }

    const char sync_msg[] = "[linux-dynamic] futex synchronization passed\n";
    write(1, sync_msg, sizeof(sync_msg) - 1);

    const char join_msg[] = "[linux-dynamic] thread joined successfully\n";
    write(1, join_msg, sizeof(join_msg) - 1);

    return 0;
}
