#define _GNU_SOURCE
#include <unistd.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <stdio.h>
#include <stdlib.h>

static int shared_val = 100;

int main(void) {
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
