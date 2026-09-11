#define _GNU_SOURCE
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
#include <string.h>
#include <stdio.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdlib.h>

const char __interp[] __attribute__((section(".interp"))) = "/lib/ld-musl-x86_64.so.1";

static void pmsg(const char *msg) {
    write(1, msg, strlen(msg));
}

#define SOCK_PATH "/tmp/scm_unrelated.sock"
#define EXPECTED_SECRET "SECRET_UNRELATED_VANTA_SCM_RIGHTS_TOKEN_987654321\n"

int main(void) {
    pmsg("[afunix-receiver] starting independent SCM_RIGHTS receiver process...\n");

    unlink(SOCK_PATH);

    int lfd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (lfd < 0) {
        pmsg("[afunix-receiver] FAIL: socket failed\n");
        return 1;
    }

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    strncpy(addr.sun_path, SOCK_PATH, sizeof(addr.sun_path) - 1);

    if (bind(lfd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        pmsg("[afunix-receiver] FAIL: bind failed\n");
        close(lfd);
        return 2;
    }

    if (listen(lfd, 5) < 0) {
        pmsg("[afunix-receiver] FAIL: listen failed\n");
        close(lfd);
        return 3;
    }

    pmsg("[afunix-receiver] listening on " SOCK_PATH ", waiting for independent sender...\n");

    int cfd = accept(lfd, NULL, NULL);
    if (cfd < 0) {
        pmsg("[afunix-receiver] FAIL: accept failed\n");
        close(lfd);
        return 4;
    }

    pmsg("[afunix-receiver] sender connected, receiving SCM_RIGHTS message...\n");

    struct msghdr rmsg = {0};
    char riov_base[32];
    struct iovec riov = { .iov_base = riov_base, .iov_len = sizeof(riov_base) };
    rmsg.msg_iov = &riov;
    rmsg.msg_iovlen = 1;

    char rcmsg_buf[CMSG_SPACE(sizeof(int))];
    memset(rcmsg_buf, 0, sizeof(rcmsg_buf));
    rmsg.msg_control = rcmsg_buf;
    rmsg.msg_controllen = sizeof(rcmsg_buf);

    ssize_t rn = recvmsg(cfd, &rmsg, 0);
    if (rn <= 0) {
        pmsg("[afunix-receiver] FAIL: recvmsg payload empty\n");
        close(cfd);
        close(lfd);
        return 5;
    }

    struct cmsghdr *rcmsg = CMSG_FIRSTHDR(&rmsg);
    if (!rcmsg || rcmsg->cmsg_level != SOL_SOCKET || rcmsg->cmsg_type != SCM_RIGHTS) {
        pmsg("[afunix-receiver] FAIL: valid SCM_RIGHTS header not received\n");
        close(cfd);
        close(lfd);
        return 6;
    }

    int received_fd = *(int *)CMSG_DATA(rcmsg);
    if (received_fd < 0) {
        pmsg("[afunix-receiver] FAIL: invalid received fd\n");
        close(cfd);
        close(lfd);
        return 7;
    }

    char file_buf[128];
    memset(file_buf, 0, sizeof(file_buf));
    ssize_t fn = read(received_fd, file_buf, sizeof(file_buf) - 1);
    close(received_fd);

    if (fn != (ssize_t)strlen(EXPECTED_SECRET) || strcmp(file_buf, EXPECTED_SECRET) != 0) {
        pmsg("[afunix-receiver] FAIL: payload read from passed fd does not match secret\n");
        close(cfd);
        close(lfd);
        return 8;
    }

    pmsg("[afunix-receiver] verified secret payload from passed fd bit-for-bit!\n");

    const char *ack = "ACK_FD_VERIFIED\n";
    write(cfd, ack, strlen(ack));

    close(cfd);
    close(lfd);
    unlink(SOCK_PATH);

    pmsg("[afunix-receiver] PASS: SCM_RIGHTS file descriptor passing across unrelated processes\n");
    return 0;
}
