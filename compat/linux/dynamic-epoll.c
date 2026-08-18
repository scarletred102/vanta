#define _GNU_SOURCE
#include <sys/epoll.h>
#include <sys/eventfd.h>
#include <unistd.h>
#include <stdio.h>
#include <stdint.h>

int main(void) {
    int epfd = epoll_create1(0);
    if (epfd < 0) {
        printf("[linux-epoll] epoll_create1 failed\n");
        return 1;
    }

    int efd = eventfd(0, 0);
    if (efd < 0) {
        printf("[linux-epoll] eventfd failed\n");
        return 2;
    }

    struct epoll_event ev;
    ev.events = EPOLLIN;
    ev.data.fd = efd;
    if (epoll_ctl(epfd, EPOLL_CTL_ADD, efd, &ev) < 0) {
        printf("[linux-epoll] epoll_ctl add failed\n");
        return 3;
    }

    uint64_t val = 1;
    if (write(efd, &val, sizeof(val)) != sizeof(val)) {
        printf("[linux-epoll] eventfd write failed\n");
        return 4;
    }

    struct epoll_event events[4];
    int n = epoll_wait(epfd, events, 4, 100);
    if (n <= 0) {
        printf("[linux-epoll] epoll_wait returned %d\n", n);
        return 5;
    }

    if (events[0].data.fd == efd && (events[0].events & EPOLLIN)) {
        printf("[linux-epoll] epoll and eventfd multiplexing verified\n");
        return 0;
    }

    return 6;
}
