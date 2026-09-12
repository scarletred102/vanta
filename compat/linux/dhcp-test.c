#define _GNU_SOURCE
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <unistd.h>
#include <string.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdlib.h>

const char __interp[] __attribute__((section(".interp"))) = "/lib/ld-musl-x86_64.so.1";

static void pmsg(const char *msg) {
    write(1, msg, strlen(msg));
}

int main(void) {
    pmsg("[dhcp-test] starting RFC 2131 DHCP client verification suite...\n");

    /* =========================================================================
     * Test 1: Read /proc/net/dhcp telemetry
     * ========================================================================= */
    int fd = open("/proc/net/dhcp", O_RDONLY);
    if (fd < 0) {
        pmsg("[dhcp-test] FAIL: unable to open /proc/net/dhcp\n");
        return 1;
    }

    char buf[512];
    memset(buf, 0, sizeof(buf));
    ssize_t n = read(fd, buf, sizeof(buf) - 1);
    close(fd);

    if (n <= 0) {
        pmsg("[dhcp-test] FAIL: /proc/net/dhcp is empty\n");
        return 2;
    }

    /* =========================================================================
     * Test 2: Verify state: BOUND
     * ========================================================================= */
    if (!strstr(buf, "state: BOUND")) {
        pmsg("[dhcp-test] FAIL: DHCP state is not BOUND\n");
        return 3;
    }
    pmsg("[dhcp-test] PASS: DHCP state BOUND verified\n");

    /* =========================================================================
     * Test 3: Verify dynamic IP lease (10.0.2.15)
     * ========================================================================= */
    if (!strstr(buf, "ip: 10.0.2.15")) {
        pmsg("[dhcp-test] FAIL: expected IP 10.0.2.15 not found in DHCP lease\n");
        return 4;
    }
    pmsg("[dhcp-test] PASS: dynamic IP lease (10.0.2.15) verified\n");

    /* =========================================================================
     * Test 4: Verify gateway and subnet mask options
     * ========================================================================= */
    if (!strstr(buf, "gateway: 10.0.2.2") || !strstr(buf, "netmask: 255.255.255.0")) {
        pmsg("[dhcp-test] FAIL: gateway or netmask option mismatched\n");
        return 5;
    }
    pmsg("[dhcp-test] PASS: subnet mask and gateway (10.0.2.2) verified\n");

    /* =========================================================================
     * Test 5: Verify DNS server option (10.0.2.3)
     * ========================================================================= */
    if (!strstr(buf, "dns: 10.0.2.3")) {
        pmsg("[dhcp-test] FAIL: DNS option 10.0.2.3 not found\n");
        return 6;
    }
    pmsg("[dhcp-test] PASS: DNS server option (10.0.2.3) verified\n");

    /* =========================================================================
     * Test 6: Verify socket binding to the dynamically leased IP (10.0.2.15)
     * ========================================================================= */
    int s = socket(AF_INET, SOCK_STREAM, 0);
    if (s < 0) {
        pmsg("[dhcp-test] FAIL: socket creation failed\n");
        return 7;
    }

    struct sockaddr_in sin;
    memset(&sin, 0, sizeof(sin));
    sin.sin_family = AF_INET;
    sin.sin_port = htons(19876);
    sin.sin_addr.s_addr = inet_addr("10.0.2.15");

    if (bind(s, (struct sockaddr *)&sin, sizeof(sin)) != 0) {
        pmsg("[dhcp-test] FAIL: bind to dynamically leased IP 10.0.2.15 failed\n");
        close(s);
        return 8;
    }

    struct sockaddr_in bound_sin;
    socklen_t bound_len = sizeof(bound_sin);
    if (getsockname(s, (struct sockaddr *)&bound_sin, &bound_len) != 0) {
        pmsg("[dhcp-test] FAIL: getsockname failed\n");
        close(s);
        return 9;
    }

    if (bound_sin.sin_addr.s_addr != inet_addr("10.0.2.15")) {
        pmsg("[dhcp-test] FAIL: getsockname IP did not match 10.0.2.15\n");
        close(s);
        return 10;
    }
    close(s);
    pmsg("[dhcp-test] PASS: socket bind to dynamically leased IP verified\n");

    /* =========================================================================
     * Test 7: Verify synthetic DHCP NAK recovery
     * ========================================================================= */
    if (!strstr(buf, "nak_retries: 1")) {
        pmsg("[dhcp-test] FAIL: expected nak_retries: 1 in /proc/net/dhcp\n");
        return 11;
    }
    pmsg("[dhcp-test] PASS: synthetic DHCP NAK recovery verified (restarted in Init and bound)\n");

    pmsg("[dhcp-test] ALL TESTS PASSED\n");
    return 0;
}
