#define _GNU_SOURCE
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/wait.h>
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

int main(void) {
    pmsg("[afunix-test] starting AF_UNIX test suite...\n");

    /* =========================================================================
     * Test 1: Stream socketpair bidirectional exchange and EOF
     * ========================================================================= */
    int sv[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, sv) < 0) {
        pmsg("[afunix-test] FAIL: socketpair(SOCK_STREAM) failed\n");
        return 10;
    }

    const char *msg_a = "HELLO STREAM A TO B";
    if (write(sv[0], msg_a, strlen(msg_a)) != (ssize_t)strlen(msg_a)) {
        pmsg("[afunix-test] FAIL: write sv[0] failed\n");
        return 11;
    }

    char buf[64];
    memset(buf, 0, sizeof(buf));
    ssize_t n = read(sv[1], buf, sizeof(buf) - 1);
    if (n != (ssize_t)strlen(msg_a) || strcmp(buf, msg_a) != 0) {
        pmsg("[afunix-test] FAIL: read sv[1] mismatched\n");
        return 12;
    }

    const char *msg_b = "HELLO STREAM B TO A";
    if (write(sv[1], msg_b, strlen(msg_b)) != (ssize_t)strlen(msg_b)) {
        pmsg("[afunix-test] FAIL: write sv[1] failed\n");
        return 13;
    }

    memset(buf, 0, sizeof(buf));
    n = read(sv[0], buf, sizeof(buf) - 1);
    if (n != (ssize_t)strlen(msg_b) || strcmp(buf, msg_b) != 0) {
        pmsg("[afunix-test] FAIL: read sv[0] mismatched\n");
        return 14;
    }

    close(sv[1]);
    n = read(sv[0], buf, sizeof(buf));
    if (n != 0) {
        pmsg("[afunix-test] FAIL: read after peer close did not return EOF (0)\n");
        return 15;
    }
    close(sv[0]);
    pmsg("[afunix-test] PASS: stream socketpair bidirectional and EOF\n");

    /* =========================================================================
     * Test 2: Datagram socketpair message boundaries
     * ========================================================================= */
    if (socketpair(AF_UNIX, SOCK_DGRAM, 0, sv) < 0) {
        pmsg("[afunix-test] FAIL: socketpair(SOCK_DGRAM) failed\n");
        return 20;
    }

    const char *dg1 = "DGRAM_MSG_ONE";
    const char *dg2 = "DGRAM_MSG_TWO";
    if (write(sv[0], dg1, strlen(dg1)) != (ssize_t)strlen(dg1)) {
        pmsg("[afunix-test] FAIL: write dg1 failed\n");
        return 21;
    }
    if (write(sv[0], dg2, strlen(dg2)) != (ssize_t)strlen(dg2)) {
        pmsg("[afunix-test] FAIL: write dg2 failed\n");
        return 22;
    }

    memset(buf, 0, sizeof(buf));
    n = read(sv[1], buf, sizeof(buf) - 1);
    if (n != (ssize_t)strlen(dg1) || strcmp(buf, dg1) != 0) {
        pmsg("[afunix-test] FAIL: datagram 1 boundary not preserved\n");
        return 23;
    }

    memset(buf, 0, sizeof(buf));
    n = read(sv[1], buf, sizeof(buf) - 1);
    if (n != (ssize_t)strlen(dg2) || strcmp(buf, dg2) != 0) {
        pmsg("[afunix-test] FAIL: datagram 2 boundary not preserved\n");
        return 24;
    }

    close(sv[0]);
    close(sv[1]);
    pmsg("[afunix-test] PASS: datagram socketpair message boundaries\n");

    /* =========================================================================
     * Test 3: Named VFS path bind and connect (/tmp/afunix.sock)
     * ========================================================================= */
    int lfd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (lfd < 0) {
        pmsg("[afunix-test] FAIL: socket(AF_UNIX, SOCK_STREAM) failed\n");
        return 30;
    }

    struct sockaddr_un saddr;
    memset(&saddr, 0, sizeof(saddr));
    saddr.sun_family = AF_UNIX;
    strncpy(saddr.sun_path, "/tmp/afunix.sock", sizeof(saddr.sun_path) - 1);

    unlink("/tmp/afunix.sock");
    if (bind(lfd, (struct sockaddr *)&saddr, sizeof(saddr)) < 0) {
        pmsg("[afunix-test] FAIL: bind(/tmp/afunix.sock) failed\n");
        return 31;
    }

    if (listen(lfd, 5) < 0) {
        pmsg("[afunix-test] FAIL: listen failed\n");
        return 32;
    }

    pid_t cpid = fork();
    if (cpid < 0) {
        pmsg("[afunix-test] FAIL: fork() failed\n");
        return 33;
    }

    if (cpid == 0) {
        /* Child: connect to listener */
        int cfd = socket(AF_UNIX, SOCK_STREAM, 0);
        if (cfd < 0) _exit(1);
        if (connect(cfd, (struct sockaddr *)&saddr, sizeof(saddr)) < 0) _exit(2);

        const char *req = "VANTA_IPC_REQ";
        if (write(cfd, req, strlen(req)) != (ssize_t)strlen(req)) _exit(3);

        char cbuf[32];
        memset(cbuf, 0, sizeof(cbuf));
        ssize_t cn = read(cfd, cbuf, sizeof(cbuf) - 1);
        if (cn != 13 || strcmp(cbuf, "VANTA_IPC_ACK") != 0) _exit(4);

        close(cfd);
        _exit(0);
    }

    /* Parent: accept connection */
    int afd = accept(lfd, NULL, NULL);
    if (afd < 0) {
        pmsg("[afunix-test] FAIL: accept() failed\n");
        return 34;
    }

    memset(buf, 0, sizeof(buf));
    n = read(afd, buf, sizeof(buf) - 1);
    if (n != 13 || strcmp(buf, "VANTA_IPC_REQ") != 0) {
        pmsg("[afunix-test] FAIL: server read request mismatched\n");
        return 35;
    }

    const char *ack = "VANTA_IPC_ACK";
    if (write(afd, ack, strlen(ack)) != (ssize_t)strlen(ack)) {
        pmsg("[afunix-test] FAIL: server write ack failed\n");
        return 36;
    }

    close(afd);
    close(lfd);

    int status = 0;
    waitpid(cpid, &status, 0);
    if (WEXITSTATUS(status) != 0) {
        pmsg("[afunix-test] FAIL: child exited with non-zero status\n");
        return 37;
    }
    pmsg("[afunix-test] PASS: named VFS path bind and connect\n");

    /* =========================================================================
     * Test 4: SCM_RIGHTS file descriptor passing across socketpair
     * ========================================================================= */
    int tfd = open("/tmp/secret.txt", O_CREAT | O_RDWR | O_TRUNC, 0644);
    if (tfd < 0) {
        pmsg("[afunix-test] FAIL: open secret file failed\n");
        return 40;
    }
    const char *secret = "SUPER_SECRET_TOKEN_42";
    if (write(tfd, secret, strlen(secret)) != (ssize_t)strlen(secret)) {
        pmsg("[afunix-test] FAIL: write secret file failed\n");
        return 41;
    }
    lseek(tfd, 0, SEEK_SET);

    if (socketpair(AF_UNIX, SOCK_STREAM, 0, sv) < 0) {
        pmsg("[afunix-test] FAIL: socketpair for SCM_RIGHTS failed\n");
        return 42;
    }

    pid_t fpid = fork();
    if (fpid < 0) {
        pmsg("[afunix-test] FAIL: fork for SCM_RIGHTS failed\n");
        return 43;
    }

    if (fpid == 0) {
        /* Child sends fd to parent */
        close(sv[0]);

        struct msghdr msg = {0};
        char iov_base[] = "FD";
        struct iovec iov = { .iov_base = iov_base, .iov_len = 2 };
        msg.msg_iov = &iov;
        msg.msg_iovlen = 1;

        char cmsg_buf[CMSG_SPACE(sizeof(int))];
        memset(cmsg_buf, 0, sizeof(cmsg_buf));
        msg.msg_control = cmsg_buf;
        msg.msg_controllen = sizeof(cmsg_buf);

        struct cmsghdr *cmsg = CMSG_FIRSTHDR(&msg);
        cmsg->cmsg_level = SOL_SOCKET;
        cmsg->cmsg_type = SCM_RIGHTS;
        cmsg->cmsg_len = CMSG_LEN(sizeof(int));
        *(int *)CMSG_DATA(cmsg) = tfd;

        if (sendmsg(sv[1], &msg, 0) < 0) {
            _exit(1);
        }
        close(tfd);
        close(sv[1]);
        _exit(0);
    }

    /* Parent receives passed fd */
    close(sv[1]);
    close(tfd); /* Close tfd in parent so parent only accesses via passed fd */

    struct msghdr rmsg = {0};
    char riov_base[16];
    struct iovec riov = { .iov_base = riov_base, .iov_len = sizeof(riov_base) };
    rmsg.msg_iov = &riov;
    rmsg.msg_iovlen = 1;

    char rcmsg_buf[CMSG_SPACE(sizeof(int))];
    memset(rcmsg_buf, 0, sizeof(rcmsg_buf));
    rmsg.msg_control = rcmsg_buf;
    rmsg.msg_controllen = sizeof(rcmsg_buf);

    ssize_t rn = recvmsg(sv[0], &rmsg, 0);
    if (rn != 2) {
        pmsg("[afunix-test] FAIL: recvmsg payload size mismatched\n");
        return 44;
    }

    struct cmsghdr *rcmsg = CMSG_FIRSTHDR(&rmsg);
    if (!rcmsg || rcmsg->cmsg_type != SCM_RIGHTS) {
        pmsg("[afunix-test] FAIL: SCM_RIGHTS header missing\n");
        return 45;
    }

    int passed_fd = *(int *)CMSG_DATA(rcmsg);
    if (passed_fd < 0) {
        pmsg("[afunix-test] FAIL: invalid passed fd\n");
        return 46;
    }

    char sbuf[64];
    memset(sbuf, 0, sizeof(sbuf));
    ssize_t sn = read(passed_fd, sbuf, sizeof(sbuf) - 1);
    if (sn != (ssize_t)strlen(secret) || strcmp(sbuf, secret) != 0) {
        pmsg("[afunix-test] FAIL: read from passed fd did not match secret content\n");
        return 47;
    }

    close(passed_fd);
    close(sv[0]);

    status = 0;
    waitpid(fpid, &status, 0);
    if (WEXITSTATUS(status) != 0) {
        pmsg("[afunix-test] FAIL: SCM_RIGHTS child failed\n");
        return 48;
    }
    pmsg("[afunix-test] PASS: SCM_RIGHTS file descriptor passing\n");

    pmsg("[afunix-test] ALL TESTS PASSED\n");
    return 0;
}
