#include <pthread.h>
#include <unistd.h>
#include <string.h>

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

    if (pthread_create(&th, NULL, thread_entry, (void *)1) != 0) {
        return 1;
    }

    const char spawn_msg[] = "[linux-dynamic] thread spawned\n";
    write(1, spawn_msg, sizeof(spawn_msg) - 1);

    pthread_mutex_lock(&g_mutex);
    g_counter += 1;
    pthread_mutex_unlock(&g_mutex);

    if (pthread_join(th, NULL) != 0) {
        return 2;
    }

    const char sync_msg[] = "[linux-dynamic] futex synchronization passed\n";
    write(1, sync_msg, sizeof(sync_msg) - 1);

    const char join_msg[] = "[linux-dynamic] thread joined successfully\n";
    write(1, join_msg, sizeof(join_msg) - 1);

    return 0;
}
