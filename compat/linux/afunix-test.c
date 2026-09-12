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

    /* =========================================================================
     * Test 5: Linux Abstract Namespace socket bind and connect
     * ========================================================================= */
    pmsg("[afunix-test] testing abstract unix socket bind and connect...\n");
    int alfd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (alfd < 0) {
        pmsg("[afunix-test] FAIL: socket for abstract bind failed\n");
        return 50;
    }

    struct sockaddr_un aaddr;
    memset(&aaddr, 0, sizeof(aaddr));
    aaddr.sun_family = AF_UNIX;
    aaddr.sun_path[0] = '\0';
    const char *abs_name = "vanta-abstract-test";
    memcpy(aaddr.sun_path + 1, abs_name, strlen(abs_name));
    socklen_t aaddr_len = sizeof(sa_family_t) + 1 + strlen(abs_name);

    if (bind(alfd, (struct sockaddr *)&aaddr, aaddr_len) < 0) {
        pmsg("[afunix-test] FAIL: bind to abstract socket failed\n");
        return 51;
    }

    if (listen(alfd, 5) < 0) {
        pmsg("[afunix-test] FAIL: listen on abstract socket failed\n");
        return 52;
    }

    /* Verify no filesystem presence: no file named "vanta-abstract-test" or in /tmp */
    if (access("/vanta-abstract-test", F_OK) == 0 || access("/tmp/vanta-abstract-test", F_OK) == 0) {
        pmsg("[afunix-test] FAIL: abstract socket created unexpected filesystem node\n");
        return 53;
    }

    /* Verify namespace collision: second bind to same abstract name must fail */
    int alfd_coll = socket(AF_UNIX, SOCK_STREAM, 0);
    if (bind(alfd_coll, (struct sockaddr *)&aaddr, aaddr_len) == 0) {
        pmsg("[afunix-test] FAIL: second bind to same abstract name unexpectedly succeeded\n");
        close(alfd_coll);
        return 54;
    }
    close(alfd_coll);

    /* Fork child to connect to abstract socket */
    pid_t apid = fork();
    if (apid < 0) {
        pmsg("[afunix-test] FAIL: fork for abstract connect failed\n");
        return 55;
    }

    if (apid == 0) {
        close(alfd);
        int cfd = socket(AF_UNIX, SOCK_STREAM, 0);
        if (cfd < 0) _exit(1);
        if (connect(cfd, (struct sockaddr *)&aaddr, aaddr_len) < 0) _exit(2);

        const char *ping = "ABSTRACT_PING";
        if (write(cfd, ping, strlen(ping)) != (ssize_t)strlen(ping)) _exit(3);

        char cbuf[32];
        memset(cbuf, 0, sizeof(cbuf));
        ssize_t cn = read(cfd, cbuf, sizeof(cbuf) - 1);
        if (cn != 13 || strcmp(cbuf, "ABSTRACT_PONG") != 0) _exit(4);

        close(cfd);
        _exit(0);
    }

    int a_conn = accept(alfd, NULL, NULL);
    if (a_conn < 0) {
        pmsg("[afunix-test] FAIL: accept on abstract socket failed\n");
        return 56;
    }

    char abuf[32];
    memset(abuf, 0, sizeof(abuf));
    ssize_t an = read(a_conn, abuf, sizeof(abuf) - 1);
    if (an != 13 || strcmp(abuf, "ABSTRACT_PING") != 0) {
        pmsg("[afunix-test] FAIL: abstract ping payload mismatched\n");
        return 57;
    }

    const char *pong = "ABSTRACT_PONG";
    if (write(a_conn, pong, strlen(pong)) != (ssize_t)strlen(pong)) {
        pmsg("[afunix-test] FAIL: abstract pong write failed\n");
        return 58;
    }

    close(a_conn);

    int astatus = 0;
    waitpid(apid, &astatus, 0);
    if (WEXITSTATUS(astatus) != 0) {
        pmsg("[afunix-test] FAIL: abstract client exited non-zero\n");
        return 59;
    }
    pmsg("[afunix-test] PASS: abstract unix socket bind and connect\n");

    close(alfd);

    /* Verify automatic release on close: re-binding to same abstract name succeeds now */
    int alfd_rebind = socket(AF_UNIX, SOCK_STREAM, 0);
    if (bind(alfd_rebind, (struct sockaddr *)&aaddr, aaddr_len) < 0) {
        pmsg("[afunix-test] FAIL: re-bind to abstract name after close failed\n");
        close(alfd_rebind);
        return 60;
    }
    close(alfd_rebind);
    pmsg("[afunix-test] PASS: abstract socket auto-release on close verified\n");

    /* =========================================================================
     * Test 6: Raw-byte sequence abstract socket with embedded null (\0abc\0def)
     * ========================================================================= */
    pmsg("[afunix-test] testing raw-byte abstract socket with embedded null...\n");
    struct sockaddr_un emb_addr;
    memset(&emb_addr, 0, sizeof(emb_addr));
    emb_addr.sun_family = AF_UNIX;
    emb_addr.sun_path[0] = '\0';
    char emb_name[7] = {'a', 'b', 'c', '\0', 'd', 'e', 'f'};
    memcpy(emb_addr.sun_path + 1, emb_name, 7);
    socklen_t emb_len = sizeof(sa_family_t) + 1 + 7;

    int emb_fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (bind(emb_fd, (struct sockaddr *)&emb_addr, emb_len) < 0) {
        pmsg("[afunix-test] FAIL: bind to embedded-null abstract socket failed\n");
        return 61;
    }

    if (listen(emb_fd, 5) < 0) {
        pmsg("[afunix-test] FAIL: listen on embedded-null abstract socket failed\n");
        return 62;
    }

    /* Verify that a truncated name "\0abc" (len 3) does NOT collide and can bind */
    struct sockaddr_un trunc_addr;
    memset(&trunc_addr, 0, sizeof(trunc_addr));
    trunc_addr.sun_family = AF_UNIX;
    trunc_addr.sun_path[0] = '\0';
    memcpy(trunc_addr.sun_path + 1, "abc", 3);
    socklen_t trunc_len = sizeof(sa_family_t) + 1 + 3;

    int trunc_fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (bind(trunc_fd, (struct sockaddr *)&trunc_addr, trunc_len) < 0) {
        pmsg("[afunix-test] FAIL: bind to truncated prefix unexpectedly collided\n");
        close(trunc_fd);
        close(emb_fd);
        return 63;
    }
    close(trunc_fd);

    /* Connect to the embedded-null name and verify data transfer */
    pid_t emb_pid = fork();
    if (emb_pid < 0) {
        pmsg("[afunix-test] FAIL: fork for embedded-null connect failed\n");
        close(emb_fd);
        return 64;
    }

    if (emb_pid == 0) {
        close(emb_fd);
        int cfd = socket(AF_UNIX, SOCK_STREAM, 0);
        if (cfd < 0) _exit(1);
        if (connect(cfd, (struct sockaddr *)&emb_addr, emb_len) < 0) _exit(2);
        if (write(cfd, "EMB_DATA_OK", 11) != 11) _exit(3);
        close(cfd);
        _exit(0);
    }

    int emb_conn = accept(emb_fd, NULL, NULL);
    if (emb_conn < 0) {
        pmsg("[afunix-test] FAIL: accept on embedded null socket failed\n");
        close(emb_fd);
        return 65;
    }

    char emb_buf[16];
    memset(emb_buf, 0, sizeof(emb_buf));
    ssize_t en = read(emb_conn, emb_buf, sizeof(emb_buf) - 1);
    close(emb_conn);
    close(emb_fd);
    int emb_status = 0;
    waitpid(emb_pid, &emb_status, 0);

    if (en != 11 || strcmp(emb_buf, "EMB_DATA_OK") != 0) {
        pmsg("[afunix-test] FAIL: data over embedded-null socket mismatched\n");
        return 66;
    }
    pmsg("[afunix-test] PASS: abstract socket raw-byte sequence with embedded null\n");

    pmsg("[afunix-test] ALL TESTS PASSED\n");
    return 0;
}
