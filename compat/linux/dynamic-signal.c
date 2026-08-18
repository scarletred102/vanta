#include <signal.h>
#include <unistd.h>
#include <string.h>

const char __interp[] __attribute__((section(".interp"))) = "/lib/ld-musl-x86_64.so.1";

static volatile int g_handled = 0;

static void signal_handler(int sig, siginfo_t *info, void *ucontext) {
    (void)info;
    (void)ucontext;
    if (sig == SIGUSR1) {
        const char msg[] = "[linux-dynamic] signal delivered and handled\n";
        write(1, msg, sizeof(msg) - 1);
        g_handled = 1;
    }
}

int main(void) {
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_sigaction = signal_handler;
    sa.sa_flags = SA_SIGINFO;
    sigemptyset(&sa.sa_mask);

    if (sigaction(SIGUSR1, &sa, NULL) != 0) {
        return 1;
    }

    const char reg_msg[] = "[linux-dynamic] signal handler registered\n";
    write(1, reg_msg, sizeof(reg_msg) - 1);

    kill(getpid(), SIGUSR1);

    if (g_handled == 1) {
        const char ret_msg[] = "[linux-dynamic] rt_sigreturn restored context\n";
        write(1, ret_msg, sizeof(ret_msg) - 1);
        return 0;
    }

    return 2;
}
