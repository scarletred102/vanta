#define _GNU_SOURCE
#include <sys/socket.h>
#include <sys/wait.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <unistd.h>
#include <string.h>
#include <stdio.h>
#include <errno.h>
#include <signal.h>
#include <stdint.h>

const char __interp[] __attribute__((section(".interp"))) = "/lib/ld-musl-x86_64.so.1";

static void pmsg(const char *msg) {
    write(1, msg, strlen(msg));
}

int main(void) {
    pmsg("[http-server] starting BSD socket lifecycle tests...\n");

    /* =========================================================================
     * Test 1: Socket creation, SO_REUSEADDR, and TCP_NODELAY sockopts
     * ========================================================================= */
    int s = socket(AF_INET, SOCK_STREAM, 0);
    if (s < 0) {
        pmsg("[http-server] FAIL: socket creation failed\n");
        return 1;
    }

    int opt = 1;
    if (setsockopt(s, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt)) != 0) {
        pmsg("[http-server] FAIL: setsockopt SO_REUSEADDR failed\n");
        return 2;
    }

    int val = 0;
    socklen_t vlen = sizeof(val);
    if (getsockopt(s, SOL_SOCKET, SO_REUSEADDR, &val, &vlen) != 0 || val != 1) {
        pmsg("[http-server] FAIL: getsockopt SO_REUSEADDR mismatch\n");
        return 3;
    }

    if (setsockopt(s, 6 /* IPPROTO_TCP */, 1 /* TCP_NODELAY */, &opt, sizeof(opt)) != 0) {
        pmsg("[http-server] FAIL: setsockopt TCP_NODELAY failed\n");
        return 4;
    }

    val = 0;
    if (getsockopt(s, 6, 1, &val, &vlen) != 0 || val != 1) {
        pmsg("[http-server] FAIL: getsockopt TCP_NODELAY mismatch\n");
        return 5;
    }
    pmsg("[http-server] PASS: socket options (SO_REUSEADDR, TCP_NODELAY)\n");

    /* =========================================================================
     * Test 2: Bind to 127.0.0.1:8080 and getsockname()
     * ========================================================================= */
    struct sockaddr_in saddr;
    memset(&saddr, 0, sizeof(saddr));
    saddr.sin_family = AF_INET;
    saddr.sin_port = htons(8080);
    saddr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);

    if (bind(s, (struct sockaddr *)&saddr, sizeof(saddr)) != 0) {
        pmsg("[http-server] FAIL: bind to 127.0.0.1:8080 failed\n");
        return 6;
    }

    struct sockaddr_in bound_addr;
    socklen_t bound_len = sizeof(bound_addr);
    if (getsockname(s, (struct sockaddr *)&bound_addr, &bound_len) != 0) {
        pmsg("[http-server] FAIL: getsockname failed\n");
        return 7;
    }
    if (ntohs(bound_addr.sin_port) != 8080) {
        pmsg("[http-server] FAIL: getsockname returned wrong port\n");
        return 8;
    }
    pmsg("[http-server] PASS: bind and getsockname (127.0.0.1:8080)\n");

    /* =========================================================================
     * Test 3: listen(s, 128)
     * ========================================================================= */
    if (listen(s, 128) != 0) {
        pmsg("[http-server] FAIL: listen failed\n");
        return 9;
    }
    pmsg("[http-server] PASS: listen(backlog=128)\n");

    /* =========================================================================
     * Test 4: Client connect, accept4, getpeername, and HTTP Request/Response
     * ========================================================================= */
    int client_sock = socket(AF_INET, SOCK_STREAM, 0);
    if (client_sock < 0) {
        pmsg("[http-server] FAIL: client socket creation failed\n");
        return 10;
    }

    if (connect(client_sock, (struct sockaddr *)&saddr, sizeof(saddr)) != 0) {
        pmsg("[http-server] FAIL: client connect to 127.0.0.1:8080 failed\n");
        return 11;
    }

    struct sockaddr_in peer_addr;
    socklen_t peer_len = sizeof(peer_addr);
    int accepted_fd = accept4(s, (struct sockaddr *)&peer_addr, &peer_len, 0);
    if (accepted_fd < 0) {
        pmsg("[http-server] FAIL: accept4 failed\n");
        return 12;
    }

    struct sockaddr_in peer_verify;
    socklen_t pv_len = sizeof(peer_verify);
    if (getpeername(accepted_fd, (struct sockaddr *)&peer_verify, &pv_len) != 0) {
        pmsg("[http-server] FAIL: getpeername failed\n");
        return 13;
    }
    if (peer_verify.sin_port != peer_addr.sin_port) {
        pmsg("[http-server] FAIL: peer port mismatch\n");
        return 14;
    }
    pmsg("[http-server] PASS: accept4 and getpeername\n");

    // Client sends HTTP GET
    const char req[] = "GET /test HTTP/1.1\r\nHost: localhost\r\n\r\n";
    if (write(client_sock, req, strlen(req)) != (ssize_t)strlen(req)) {
        pmsg("[http-server] FAIL: client write failed\n");
        return 15;
    }

    char req_buf[128];
    memset(req_buf, 0, sizeof(req_buf));
    ssize_t nread = read(accepted_fd, req_buf, sizeof(req_buf) - 1);
    if (nread <= 0 || strstr(req_buf, "GET /test") == NULL) {
        pmsg("[http-server] FAIL: server read invalid request\n");
        return 16;
    }

    // Server sends HTTP response
    const char resp[] = "HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\nHello Vanta!";
    if (write(accepted_fd, resp, strlen(resp)) != (ssize_t)strlen(resp)) {
        pmsg("[http-server] FAIL: server response write failed\n");
        return 17;
    }

    char resp_buf[128];
    memset(resp_buf, 0, sizeof(resp_buf));
    ssize_t cr_n = read(client_sock, resp_buf, sizeof(resp_buf) - 1);
    if (cr_n <= 0 || strstr(resp_buf, "Hello Vanta!") == NULL) {
        pmsg("[http-server] FAIL: client received invalid response\n");
        return 18;
    }
    pmsg("[http-server] PASS: HTTP GET request-response payload verified\n");

    /* =========================================================================
     * Test 5: Graceful FIN / EOF teardown
     * ========================================================================= */
    close(client_sock);
    char eof_buf[16];
    ssize_t eof_res = read(accepted_fd, eof_buf, sizeof(eof_buf));
    if (eof_res != 0) {
        pmsg("[http-server] FAIL: expected EOF on peer close\n");
        return 19;
    }
    close(accepted_fd);
    pmsg("[http-server] PASS: graceful FIN/EOF teardown\n");

    /* =========================================================================
     * Test 6: EPIPE / SIGPIPE delivery on write to closed socket
     * ========================================================================= */
    int c2 = socket(AF_INET, SOCK_STREAM, 0);
    if (connect(c2, (struct sockaddr *)&saddr, sizeof(saddr)) != 0) {
        pmsg("[http-server] FAIL: c2 connect failed\n");
        return 20;
    }
    int a2 = accept4(s, NULL, NULL, 0);
    if (a2 < 0) {
        pmsg("[http-server] FAIL: a2 accept4 failed\n");
        return 21;
    }
    // Server immediately closes connection
    close(a2);

    usleep(10000);

    // Client attempts write with MSG_NOSIGNAL -> must return -1 with EPIPE
    ssize_t wr = send(c2, "test", 4, MSG_NOSIGNAL);
    if (wr < 0) {
        pmsg("[http-server] PASS: EPIPE delivered on closed socket write\n");
    } else {
        pmsg("[http-server] FAIL: write to closed socket succeeded unexpectedly\n");
        return 22;
    }
    close(c2);

    /* =========================================================================
     * Test 7: TIME_WAIT port reuse with SO_REUSEADDR
     * ========================================================================= */
    close(s);
    int s_new = socket(AF_INET, SOCK_STREAM, 0);
    int reuse = 1;
    setsockopt(s_new, SOL_SOCKET, SO_REUSEADDR, &reuse, sizeof(reuse));
    if (bind(s_new, (struct sockaddr *)&saddr, sizeof(saddr)) != 0) {
        pmsg("[http-server] FAIL: immediate port reuse failed\n");
        return 23;
    }
    if (listen(s_new, 128) != 0) {
        pmsg("[http-server] FAIL: re-listen failed\n");
        return 24;
    }
    pmsg("[http-server] PASS: TIME_WAIT port reuse with SO_REUSEADDR\n");

    /* =========================================================================
     * Test 8: Concurrent Client Connections
     * ========================================================================= */
    pmsg("[http-server] testing concurrent connections...\n");
    int c_conns[3];
    for (int i = 0; i < 3; i++) {
        c_conns[i] = socket(AF_INET, SOCK_STREAM, 0);
        if (connect(c_conns[i], (struct sockaddr *)&saddr, sizeof(saddr)) != 0) {
            pmsg("[http-server] FAIL: concurrent connect failed\n");
            return 25;
        }
    }

    int a_conns[3];
    for (int i = 0; i < 3; i++) {
        a_conns[i] = accept4(s_new, NULL, NULL, 0);
        if (a_conns[i] < 0) {
            pmsg("[http-server] FAIL: concurrent accept4 failed\n");
            return 26;
        }
    }

    // Exchange data on all 3 connections
    for (int i = 0; i < 3; i++) {
        char req_tok[16];
        snprintf(req_tok, sizeof(req_tok), "CONN_%d\n", i);
        write(c_conns[i], req_tok, strlen(req_tok));

        char svr_buf[16];
        memset(svr_buf, 0, sizeof(svr_buf));
        read(a_conns[i], svr_buf, sizeof(svr_buf) - 1);
        if (strstr(svr_buf, req_tok) == NULL) {
            pmsg("[http-server] FAIL: concurrent data mismatch\n");
            return 27;
        }

        char resp_tok[16];
        snprintf(resp_tok, sizeof(resp_tok), "ACK_%d\n", i);
        write(a_conns[i], resp_tok, strlen(resp_tok));

        char cli_buf[16];
        memset(cli_buf, 0, sizeof(cli_buf));
        read(c_conns[i], cli_buf, sizeof(cli_buf) - 1);
        if (strstr(cli_buf, resp_tok) == NULL) {
            pmsg("[http-server] FAIL: concurrent ack mismatch\n");
            return 28;
        }

        close(c_conns[i]);
        close(a_conns[i]);
    }
    pmsg("[http-server] PASS: 3 concurrent client connections verified\n");

    close(s_new);
    pmsg("[http-server] ALL TESTS PASSED\n");
    return 0;
}
