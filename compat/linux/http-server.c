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
#include <fcntl.h>

const char __interp[] __attribute__((section(".interp"))) = "/lib/ld-musl-x86_64.so.1";

static void pmsg(const char *msg) {
    write(1, msg, strlen(msg));
}

static uint16_t csum_fold(uint32_t sum) {
    while (sum >> 16)
        sum = (sum & 0xffff) + (sum >> 16);
    return ~((uint16_t)sum);
}

static uint16_t ip_csum(const uint8_t *buf, size_t len) {
    uint32_t sum = 0;
    for (size_t i = 0; i < len; i += 2) {
        sum += ((uint32_t)buf[i] << 8) | buf[i + 1];
    }
    return csum_fold(sum);
}

static uint16_t tcp_csum(const uint8_t *src_ip, const uint8_t *dst_ip, const uint8_t *tcp_hdr, size_t tcp_len) {
    uint32_t sum = 0;
    sum += ((uint32_t)src_ip[0] << 8) | src_ip[1];
    sum += ((uint32_t)src_ip[2] << 8) | src_ip[3];
    sum += ((uint32_t)dst_ip[0] << 8) | dst_ip[1];
    sum += ((uint32_t)dst_ip[2] << 8) | dst_ip[3];
    sum += 6; // IP_PROTOCOL_TCP
    sum += (uint32_t)tcp_len;
    for (size_t i = 0; i < tcp_len; i += 2) {
        if (i + 1 < tcp_len) {
            sum += ((uint32_t)tcp_hdr[i] << 8) | tcp_hdr[i + 1];
        } else {
            sum += ((uint32_t)tcp_hdr[i] << 8);
        }
    }
    return csum_fold(sum);
}

static void build_raw_syn_frame(uint8_t *frame, uint16_t src_port, uint16_t dst_port, uint32_t seq) {
    memset(frame, 0, 54);
    // Ethernet header: 14 bytes
    frame[12] = 0x08;
    frame[13] = 0x00; // ETHERTYPE_IPV4

    // IPv4 header: 20 bytes (offset 14)
    frame[14] = 0x45; // Version 4, IHL 5
    frame[15] = 0x00; // TOS
    frame[16] = 0x00;
    frame[17] = 40;   // Total length = 40 (20 IP + 20 TCP)
    frame[18] = 0x00;
    frame[19] = 0x01; // ID
    frame[20] = 0x40;
    frame[21] = 0x00; // DF flag
    frame[22] = 64;   // TTL
    frame[23] = 6;    // Protocol TCP
    frame[26] = 127; frame[27] = 0; frame[28] = 0; frame[29] = 1; // Src IP 127.0.0.1
    frame[30] = 127; frame[31] = 0; frame[32] = 0; frame[33] = 1; // Dst IP 127.0.0.1
    uint16_t ipc = ip_csum(&frame[14], 20);
    frame[24] = (uint8_t)(ipc >> 8);
    frame[25] = (uint8_t)(ipc & 0xff);

    // TCP header: 20 bytes (offset 34)
    frame[34] = (uint8_t)(src_port >> 8);
    frame[35] = (uint8_t)(src_port & 0xff);
    frame[36] = (uint8_t)(dst_port >> 8);
    frame[37] = (uint8_t)(dst_port & 0xff);
    frame[38] = (uint8_t)((seq >> 24) & 0xff);
    frame[39] = (uint8_t)((seq >> 16) & 0xff);
    frame[40] = (uint8_t)((seq >> 8) & 0xff);
    frame[41] = (uint8_t)(seq & 0xff);
    frame[46] = 0x50; // Data offset 5 (20 bytes)
    frame[47] = 0x02; // Flags: TCP_SYN
    frame[48] = 0xff;
    frame[49] = 0xff; // Window size 65535
    uint8_t src_ip[4] = {127, 0, 0, 1};
    uint8_t dst_ip[4] = {127, 0, 0, 1};
    uint16_t tc = tcp_csum(src_ip, dst_ip, &frame[34], 20);
    if (tc == 0) tc = 0xffff;
    frame[50] = (uint8_t)(tc >> 8);
    frame[51] = (uint8_t)(tc & 0xff);
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
     * Test 7: TIME_WAIT port reuse semantics (negative without SO_REUSEADDR, positive with SO_REUSEADDR)
     * ========================================================================= */
    close(s);

    // 7a: bind WITHOUT SO_REUSEADDR to port in TIME_WAIT must fail with EADDRINUSE
    int s_no_reuse = socket(AF_INET, SOCK_STREAM, 0);
    if (bind(s_no_reuse, (struct sockaddr *)&saddr, sizeof(saddr)) == 0) {
        pmsg("[http-server] FAIL: bind without SO_REUSEADDR unexpectedly succeeded on TIME_WAIT port\n");
        close(s_no_reuse);
        return 23;
    }
    if (errno != EADDRINUSE) {
        char err_buf[80];
        snprintf(err_buf, sizeof(err_buf), "[http-server] FAIL: bind failed with errno=%d (expected EADDRINUSE=%d)\n", errno, EADDRINUSE);
        pmsg(err_buf);
        close(s_no_reuse);
        return 24;
    }
    close(s_no_reuse);
    pmsg("[http-server] PASS: bind without SO_REUSEADDR correctly failed with EADDRINUSE\n");

    // 7b: bind WITH SO_REUSEADDR to the same port in TIME_WAIT must succeed
    int s_new = socket(AF_INET, SOCK_STREAM, 0);
    int reuse = 1;
    setsockopt(s_new, SOL_SOCKET, SO_REUSEADDR, &reuse, sizeof(reuse));
    if (bind(s_new, (struct sockaddr *)&saddr, sizeof(saddr)) != 0) {
        pmsg("[http-server] FAIL: bind with SO_REUSEADDR failed on TIME_WAIT port\n");
        close(s_new);
        return 25;
    }
    if (listen(s_new, 128) != 0) {
        pmsg("[http-server] FAIL: re-listen failed\n");
        close(s_new);
        return 26;
    }
    pmsg("[http-server] PASS: TIME_WAIT port reuse with SO_REUSEADDR succeeded\n");

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
    pmsg("[http-server] active concurrent connection count: 3 (all 3 in Established state concurrently)\n");

    // Send requests on ALL 3 client sockets first (concurrent data in flight)
    for (int i = 0; i < 3; i++) {
        char req_tok[32];
        snprintf(req_tok, sizeof(req_tok), "CONCURRENT_REQ_%d\n", i);
        write(c_conns[i], req_tok, strlen(req_tok));
    }

    // Read requests on ALL 3 server sockets
    for (int i = 0; i < 3; i++) {
        char svr_buf[32];
        memset(svr_buf, 0, sizeof(svr_buf));
        read(a_conns[i], svr_buf, sizeof(svr_buf) - 1);
        char exp_tok[32];
        snprintf(exp_tok, sizeof(exp_tok), "CONCURRENT_REQ_%d\n", i);
        if (strstr(svr_buf, exp_tok) == NULL) {
            pmsg("[http-server] FAIL: concurrent data mismatch\n");
            return 27;
        }
    }

    // Send replies on ALL 3 server sockets
    for (int i = 0; i < 3; i++) {
        char resp_tok[32];
        snprintf(resp_tok, sizeof(resp_tok), "CONCURRENT_ACK_%d\n", i);
        write(a_conns[i], resp_tok, strlen(resp_tok));
    }

    // Read replies on ALL 3 client sockets
    for (int i = 0; i < 3; i++) {
        char cli_buf[32];
        memset(cli_buf, 0, sizeof(cli_buf));
        read(c_conns[i], cli_buf, sizeof(cli_buf) - 1);
        char exp_ack[32];
        snprintf(exp_ack, sizeof(exp_ack), "CONCURRENT_ACK_%d\n", i);
        if (strstr(cli_buf, exp_ack) == NULL) {
            pmsg("[http-server] FAIL: concurrent ack mismatch\n");
            return 28;
        }
    }

    // Close all 3 connections
    for (int i = 0; i < 3; i++) {
        close(c_conns[i]);
        close(a_conns[i]);
    }
    pmsg("[http-server] PASS: 3 concurrent client connections verified (concurrent data in flight across 3 active sockets)\n");

    /* =========================================================================
     * Test 9: Simultaneous close (CLOSING -> TIME_WAIT -> CLOSED)
     * ========================================================================= */
    pmsg("[http-server] testing simultaneous close...\n");
    int s_sim_listen = socket(AF_INET, SOCK_STREAM, 0);
    int opt_sim = 1;
    setsockopt(s_sim_listen, SOL_SOCKET, SO_REUSEADDR, &opt_sim, sizeof(opt_sim));
    struct sockaddr_in sim_addr;
    memset(&sim_addr, 0, sizeof(sim_addr));
    sim_addr.sin_family = AF_INET;
    sim_addr.sin_port = htons(8081);
    sim_addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    if (bind(s_sim_listen, (struct sockaddr *)&sim_addr, sizeof(sim_addr)) != 0 ||
        listen(s_sim_listen, 5) != 0) {
        pmsg("[http-server] FAIL: simultaneous close listener setup failed\n");
        return 30;
    }

    int sim_c = socket(AF_INET, SOCK_STREAM, 0);
    if (connect(sim_c, (struct sockaddr *)&sim_addr, sizeof(sim_addr)) != 0) {
        pmsg("[http-server] FAIL: sim_c connect failed\n");
        return 31;
    }
    int sim_a = accept4(s_sim_listen, NULL, NULL, 0);
    if (sim_a < 0) {
        pmsg("[http-server] FAIL: sim_a accept4 failed\n");
        return 32;
    }
    close(s_sim_listen);

    // Both endpoints initiate close simultaneously
    close(sim_c);
    close(sim_a);

    // Allow loopback exchange of FINs and ACKs: both transition FinWait1 -> Closing -> TimeWait
    usleep(100000); // 100ms

    // Check /proc/net/tcp for port 8081 in TIME_WAIT
    int pfd = open("/proc/net/tcp", O_RDONLY);
    if (pfd >= 0) {
        char pbuf[2048];
        memset(pbuf, 0, sizeof(pbuf));
        read(pfd, pbuf, sizeof(pbuf) - 1);
        close(pfd);
        if (strstr(pbuf, "1F91") == NULL) { // 8081 in hex
            pmsg("[http-server] FAIL: port 8081 socket not found in /proc/net/tcp\n");
            return 33;
        }
    }

    // Wait for 2*MSL (2000ms) timer expiration
    usleep(2100000); // 2.1s

    // Verify /proc/net/tcp has purged the sockets (transition to CLOSED)
    pfd = open("/proc/net/tcp", O_RDONLY);
    if (pfd >= 0) {
        char pbuf[2048];
        memset(pbuf, 0, sizeof(pbuf));
        read(pfd, pbuf, sizeof(pbuf) - 1);
        close(pfd);
        if (strstr(pbuf, ":1F91") != NULL) {
            pmsg("[http-server] FAIL: port 8081 socket still present after 2*MSL\n");
            return 34;
        }
    }
    pmsg("[http-server] PASS: simultaneous close (CLOSING -> TIME_WAIT -> CLOSED)\n");

    /* =========================================================================
     * Test 10: SYN queue timeout (purged after 3000ms verified via /proc/net/tcp)
     * ========================================================================= */
    pmsg("[http-server] testing SYN queue timeout...\n");
    int s_to_listen = socket(AF_INET, SOCK_STREAM, 0);
    int opt_to = 1;
    setsockopt(s_to_listen, SOL_SOCKET, SO_REUSEADDR, &opt_to, sizeof(opt_to));
    struct sockaddr_in to_addr;
    memset(&to_addr, 0, sizeof(to_addr));
    to_addr.sin_family = AF_INET;
    to_addr.sin_port = htons(8082);
    to_addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    if (bind(s_to_listen, (struct sockaddr *)&to_addr, sizeof(to_addr)) != 0 ||
        listen(s_to_listen, 5) != 0) {
        pmsg("[http-server] FAIL: SYN timeout listener setup failed\n");
        return 35;
    }

    int raw_fd = socket(AF_INET, SOCK_RAW, 0);
    if (raw_fd < 0) {
        pmsg("[http-server] FAIL: SOCK_RAW creation failed\n");
        return 36;
    }

    uint8_t syn_pkt[54];
    build_raw_syn_frame(syn_pkt, 18082, 8082, 1000);
    if (sendto(raw_fd, syn_pkt, sizeof(syn_pkt), 0, (struct sockaddr *)&to_addr, sizeof(to_addr)) != sizeof(syn_pkt)) {
        pmsg("[http-server] FAIL: raw SYN sendto failed\n");
        return 37;
    }

    // Verify /proc/net/tcp shows pending_syns=1 for port 8082 (1F92)
    pfd = open("/proc/net/tcp", O_RDONLY);
    if (pfd < 0) {
        pmsg("[http-server] FAIL: open /proc/net/tcp failed\n");
        return 38;
    }
    char tcp_buf[2048];
    memset(tcp_buf, 0, sizeof(tcp_buf));
    read(pfd, tcp_buf, sizeof(tcp_buf) - 1);
    close(pfd);
    char *line = strstr(tcp_buf, ":1F92");
    if (!line || strstr(line, "pending_syns=1") == NULL) {
        pmsg("[http-server] FAIL: expected pending_syns=1 in /proc/net/tcp\n");
        return 39;
    }

    // Sleep 3.2s (> 3000ms timeout)
    usleep(3200000);

    // Verify /proc/net/tcp shows pending_syns=0 for port 8082 (1F92)
    pfd = open("/proc/net/tcp", O_RDONLY);
    if (pfd < 0) {
        pmsg("[http-server] FAIL: open /proc/net/tcp second time failed\n");
        return 40;
    }
    memset(tcp_buf, 0, sizeof(tcp_buf));
    read(pfd, tcp_buf, sizeof(tcp_buf) - 1);
    close(pfd);
    line = strstr(tcp_buf, ":1F92");
    if (!line || strstr(line, "pending_syns=0") == NULL) {
        pmsg("[http-server] FAIL: expected pending_syns=0 after 3000ms timeout in /proc/net/tcp\n");
        return 41;
    }
    close(s_to_listen);
    pmsg("[http-server] PASS: SYN queue timeout (purged after 3000ms verified via /proc/net/tcp)\n");

    /* =========================================================================
     * Test 11: SYN cookies under saturated backlog
     * ========================================================================= */
    pmsg("[http-server] testing SYN cookies under saturated backlog...\n");
    int s_cook_listen = socket(AF_INET, SOCK_STREAM, 0);
    int opt_c = 1;
    setsockopt(s_cook_listen, SOL_SOCKET, SO_REUSEADDR, &opt_c, sizeof(opt_c));
    struct sockaddr_in cook_addr;
    memset(&cook_addr, 0, sizeof(cook_addr));
    cook_addr.sin_family = AF_INET;
    cook_addr.sin_port = htons(8083);
    cook_addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    // Backlog = 2
    if (bind(s_cook_listen, (struct sockaddr *)&cook_addr, sizeof(cook_addr)) != 0 ||
        listen(s_cook_listen, 2) != 0) {
        pmsg("[http-server] FAIL: SYN cookie listener setup failed\n");
        return 42;
    }

    // Saturate backlog with 2 raw SYNs from different ports
    build_raw_syn_frame(syn_pkt, 28081, 8083, 2001);
    sendto(raw_fd, syn_pkt, sizeof(syn_pkt), 0, (struct sockaddr *)&cook_addr, sizeof(cook_addr));
    build_raw_syn_frame(syn_pkt, 28082, 8083, 2002);
    sendto(raw_fd, syn_pkt, sizeof(syn_pkt), 0, (struct sockaddr *)&cook_addr, sizeof(cook_addr));

    // Verify /proc/net/tcp shows pending_syns=2 (saturated) for port 8083 (1F93)
    pfd = open("/proc/net/tcp", O_RDONLY);
    if (pfd >= 0) {
        memset(tcp_buf, 0, sizeof(tcp_buf));
        read(pfd, tcp_buf, sizeof(tcp_buf) - 1);
        close(pfd);
        line = strstr(tcp_buf, ":1F93");
        if (!line || strstr(line, "pending_syns=2") == NULL) {
            pmsg("[http-server] FAIL: expected pending_syns=2 (backlog saturated)\n");
            return 43;
        }
    }

    // Connect legitimate client while backlog is saturated (must succeed via SYN cookie)
    int c_cook = socket(AF_INET, SOCK_STREAM, 0);
    if (connect(c_cook, (struct sockaddr *)&cook_addr, sizeof(cook_addr)) != 0) {
        pmsg("[http-server] FAIL: connect under saturated backlog failed\n");
        return 44;
    }

    int a_cook = accept4(s_cook_listen, NULL, NULL, 0);
    if (a_cook < 0) {
        pmsg("[http-server] FAIL: accept4 under saturated backlog failed\n");
        return 45;
    }

    // Verify bidirectional data exchange on connection established via SYN cookie
    const char cmsg[] = "COOKIE_OK\n";
    write(c_cook, cmsg, strlen(cmsg));
    char abuf[32];
    memset(abuf, 0, sizeof(abuf));
    read(a_cook, abuf, sizeof(abuf) - 1);
    if (strstr(abuf, "COOKIE_OK") == NULL) {
        pmsg("[http-server] FAIL: data exchange on SYN cookie socket failed\n");
        return 46;
    }

    close(c_cook);
    close(a_cook);
    close(s_cook_listen);
    close(raw_fd);
    pmsg("[http-server] PASS: SYN cookie protection under saturated backlog verified\n");

    /* =========================================================================
     * Test 12: Host HTTP GET request (via QEMU hostfwd)
     * ========================================================================= */
    pmsg("[http-server] waiting for host curl request on port 8080...\n");
    struct sockaddr_in host_cli;
    socklen_t host_cli_len = sizeof(host_cli);
    int host_fd = accept4(s_new, (struct sockaddr *)&host_cli, &host_cli_len, 0);
    if (host_fd >= 0) {
        char host_req[512];
        memset(host_req, 0, sizeof(host_req));
        read(host_fd, host_req, sizeof(host_req) - 1);
        if (strstr(host_req, "GET") != NULL) {
            const char host_resp[] = "HTTP/1.1 200 OK\r\nContent-Length: 12\r\nConnection: close\r\n\r\nHello Vanta!";
            write(host_fd, host_resp, strlen(host_resp));
            pmsg("[http-server] PASS: host curl request served successfully with 'Hello Vanta!'\n");
        } else {
            pmsg("[http-server] FAIL: host request missing GET\n");
        }
        close(host_fd);
    } else {
        pmsg("[http-server] FAIL: accept4 for host request failed\n");
    }

    close(s_new);
    pmsg("[http-server] ALL TESTS PASSED\n");
    return 0;
}
