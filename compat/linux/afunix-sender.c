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
#define FILE_PATH "/tmp/scm_secret.txt"
#define SECRET_PAYLOAD "SECRET_UNRELATED_VANTA_SCM_RIGHTS_TOKEN_987654321\n"

int main(void) {
    pmsg("[afunix-sender] starting independent SCM_RIGHTS sender process...\n");

    /* Create and write file to pass */
    int tfd = open(FILE_PATH, O_RDWR | O_CREAT | O_TRUNC, 0644);
    if (tfd < 0) {
        pmsg("[afunix-sender] FAIL: open file failed\n");
        return 1;
    }
    if (write(tfd, SECRET_PAYLOAD, strlen(SECRET_PAYLOAD)) != (ssize_t)strlen(SECRET_PAYLOAD)) {
        pmsg("[afunix-sender] FAIL: write secret file failed\n");
        close(tfd);
        return 2;
    }
    lseek(tfd, 0, SEEK_SET);

    /* Connect to receiver */
    int sfd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (sfd < 0) {
        pmsg("[afunix-sender] FAIL: socket failed\n");
        close(tfd);
        return 3;
    }

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    strncpy(addr.sun_path, SOCK_PATH, sizeof(addr.sun_path) - 1);

    int connected = 0;
    for (int retry = 0; retry < 50; retry++) {
        if (connect(sfd, (struct sockaddr *)&addr, sizeof(addr)) == 0) {
            connected = 1;
            break;
        }
        usleep(10000); // 10ms retry
    }

    if (!connected) {
        pmsg("[afunix-sender] FAIL: unable to connect to receiver socket\n");
        close(tfd);
        close(sfd);
        return 4;
    }

    pmsg("[afunix-sender] connected to receiver, sending file descriptor via SCM_RIGHTS...\n");

    struct msghdr msg = {0};
    char iov_base[] = "PASSING_FD";
    struct iovec iov = { .iov_base = iov_base, .iov_len = sizeof(iov_base) };
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

    if (sendmsg(sfd, &msg, 0) < 0) {
        pmsg("[afunix-sender] FAIL: sendmsg SCM_RIGHTS failed\n");
        close(tfd);
        close(sfd);
        return 5;
    }

    /* Immediately close original file descriptor in sender */
    close(tfd);
    unlink(FILE_PATH);

    /* Wait for acknowledgement from receiver */
    char ack_buf[32];
    memset(ack_buf, 0, sizeof(ack_buf));
    ssize_t an = read(sfd, ack_buf, sizeof(ack_buf) - 1);
    close(sfd);

    if (an <= 0 || strstr(ack_buf, "ACK_FD_VERIFIED") == NULL) {
        pmsg("[afunix-sender] FAIL: did not receive ACK from receiver\n");
        return 6;
    }

    pmsg("[afunix-sender] PASS: receiver confirmed verification of passed fd\n");
    return 0;
}
