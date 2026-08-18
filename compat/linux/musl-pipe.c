#include <unistd.h>
#include <sys/uio.h>
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

    static const char msg[] = "[linux-musl] pipes and descriptors passed\n";
    write(1, msg, sizeof(msg) - 1);
    return 0;
}
