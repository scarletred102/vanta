#include <unistd.h>
#include <sys/uio.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <string.h>

int main(void) {
    int fds[2];
    if (pipe(fds) != 0) {
        return 1;
    }

    // Duplicate reader to fd 10
    if (dup2(fds[0], 10) < 0) {
        close(fds[0]);
        close(fds[1]);
        return 2;
    }
    close(fds[0]);

    // Test writev
    struct iovec iov[2];
    char part1[] = "hello ";
    char part2[] = "pipe\n";
    iov[0].iov_base = part1;
    iov[0].iov_len = sizeof(part1) - 1;
    iov[1].iov_base = part2;
    iov[1].iov_len = sizeof(part2) - 1;

    ssize_t written = writev(fds[1], iov, 2);
    if (written != (ssize_t)(iov[0].iov_len + iov[1].iov_len)) {
        close(fds[1]);
        close(10);
        return 3;
    }
    close(fds[1]);

    char buf[32];
    memset(buf, 0, sizeof(buf));
    ssize_t read_bytes = read(10, buf, sizeof(buf) - 1);
    close(10);

    if (read_bytes != written || strcmp(buf, "hello pipe\n") != 0) {
        return 4;
    }

    // Phase 2: Pipe throughput stress test (64 KB across parent/child)
    int stress_pipe[2];
    if (pipe(stress_pipe) != 0) {
        return 5;
    }
    pid_t p = fork();
    if (p < 0) {
        return 6;
    }
    if (p == 0) {
        close(stress_pipe[0]);
        char chunk[4096];
        for (int i = 0; i < 16; i++) {
            memset(chunk, (char)(i + 0x41), sizeof(chunk));
            ssize_t w = write(stress_pipe[1], chunk, sizeof(chunk));
            if (w != (ssize_t)sizeof(chunk)) {
                _exit(20);
            }
        }
        close(stress_pipe[1]);
        _exit(0);
    } else {
        close(stress_pipe[1]);
        char recv_buf[4096];
        size_t total_read = 0;
        while (1) {
            ssize_t r = read(stress_pipe[0], recv_buf, sizeof(recv_buf));
            if (r < 0) {
                close(stress_pipe[0]);
                return 7;
            }
            if (r == 0) break;
            total_read += (size_t)r;
        }
        close(stress_pipe[0]);
        int status = 0;
        waitpid(p, &status, 0);
        if (total_read != 16 * 4096 || WEXITSTATUS(status) != 0) {
            return 8;
        }
        static const char stress_msg[] = "[linux-musl] pipe throughput 64KB passed\n";
        write(1, stress_msg, sizeof(stress_msg) - 1);
    }

    static const char msg[] = "[linux-musl] pipes and descriptors passed\n";
    write(1, msg, sizeof(msg) - 1);
    return 0;
}
