#define _GNU_SOURCE
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <netdb.h>
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

static uint64_t get_proc_dns_queries(void) {
    int fd = open("/proc/net/dns", O_RDONLY);
    if (fd < 0) {
        return 0;
    }
    char buf[256];
    memset(buf, 0, sizeof(buf));
    ssize_t n = read(fd, buf, sizeof(buf) - 1);
    close(fd);
    if (n <= 0) return 0;

    const char *q = strstr(buf, "queries: ");
    if (!q) return 0;
    return (uint64_t)strtoull(q + 9, NULL, 10);
}

int main(void) {
    pmsg("[dns-test] starting RFC 1035 DNS resolver test suite...\n");

    /* =========================================================================
     * Test 1: Static resolution (localhost -> 127.0.0.1)
     * ========================================================================= */
    struct addrinfo *res = NULL;
    int rc = getaddrinfo("localhost", NULL, NULL, &res);
    if (rc != 0 || res == NULL) {
        pmsg("[dns-test] FAIL: localhost getaddrinfo failed\n");
        return 1;
    }
    struct sockaddr_in *sin = (struct sockaddr_in *)res->ai_addr;
    if (sin->sin_addr.s_addr != inet_addr("127.0.0.1")) {
        pmsg("[dns-test] FAIL: localhost did not resolve to 127.0.0.1\n");
        return 2;
    }
    freeaddrinfo(res);
    res = NULL;
    pmsg("[dns-test] PASS: static resolution (localhost -> 127.0.0.1)\n");

    /* =========================================================================
     * Test 2: Known hostname positive lookup (example.com)
     * ========================================================================= */
    uint64_t q_before = get_proc_dns_queries();

    rc = getaddrinfo("example.com", NULL, NULL, &res);
    if (rc != 0 || res == NULL) {
        pmsg("[dns-test] FAIL: example.com getaddrinfo failed\n");
        return 3;
    }
    sin = (struct sockaddr_in *)res->ai_addr;
    uint32_t ip1 = sin->sin_addr.s_addr;
    if (ip1 == 0 || ip1 == INADDR_NONE) {
        pmsg("[dns-test] FAIL: invalid IP returned for example.com\n");
        return 4;
    }

    uint64_t q_after1 = get_proc_dns_queries();
    if (q_after1 != q_before + 1) {
        pmsg("[dns-test] FAIL: expected exactly 1 outbound query on cache miss\n");
        return 5;
    }

    char ip_str[32];
    inet_ntop(AF_INET, &sin->sin_addr, ip_str, sizeof(ip_str));
    char log_buf[128];
    snprintf(log_buf, sizeof(log_buf), "[dns-test] PASS: positive lookup (example.com -> %s)\n", ip_str);
    pmsg(log_buf);

    /* =========================================================================
     * Test 3: TTL Cache Hit (second lookup within TTL emits 0 outbound packets)
     * ========================================================================= */
    struct addrinfo *res2 = NULL;
    rc = getaddrinfo("example.com", NULL, NULL, &res2);
    if (rc != 0 || res2 == NULL) {
        pmsg("[dns-test] FAIL: second example.com lookup failed\n");
        return 6;
    }
    struct sockaddr_in *sin2 = (struct sockaddr_in *)res2->ai_addr;
    int found_match = 0;
    for (struct addrinfo *p = res; p != NULL; p = p->ai_next) {
        struct sockaddr_in *p_sin = (struct sockaddr_in *)p->ai_addr;
        if (p_sin->sin_addr.s_addr == sin2->sin_addr.s_addr) {
            found_match = 1;
            break;
        }
    }
    if (!found_match) {
        pmsg("[dns-test] FAIL: cached IP does not match first lookup\n");
        return 7;
    }

    uint64_t q_after2 = get_proc_dns_queries();
    if (q_after2 != q_after1) {
        pmsg("[dns-test] FAIL: outbound query count increased during TTL cache hit\n");
        return 8;
    }
    freeaddrinfo(res2);
    res2 = NULL;
    pmsg("[dns-test] PASS: TTL cache hit (0 outbound packets on second lookup)\n");

    /* =========================================================================
     * Test 4: gethostbyname() API support
     * ========================================================================= */
    struct hostent *he = gethostbyname("example.com");
    if (he == NULL || he->h_addr_list == NULL || he->h_addr_list[0] == NULL) {
        pmsg("[dns-test] FAIL: gethostbyname failed for example.com\n");
        return 9;
    }
    int he_match = 0;
    for (int i = 0; he->h_addr_list[i] != NULL; i++) {
        uint32_t he_ip = *(uint32_t *)he->h_addr_list[i];
        for (struct addrinfo *p = res; p != NULL; p = p->ai_next) {
            struct sockaddr_in *p_sin = (struct sockaddr_in *)p->ai_addr;
            if (p_sin->sin_addr.s_addr == he_ip) {
                he_match = 1;
                break;
            }
        }
        if (he_match) break;
    }
    if (!he_match) {
        pmsg("[dns-test] FAIL: gethostbyname returned mismatched IP\n");
        return 10;
    }
    pmsg("[dns-test] PASS: gethostbyname(example.com) verified\n");

    /* =========================================================================
     * Test 5: NXDOMAIN for nonexistent domain
     * ========================================================================= */
    struct addrinfo *res3 = NULL;
    rc = getaddrinfo("this-nonexistent-domain-xyz987654.invalid", NULL, NULL, &res3);
    if (rc == 0 || res3 != NULL) {
        pmsg("[dns-test] FAIL: nonexistent domain unexpectedly resolved\n");
        if (res3) freeaddrinfo(res3);
        return 11;
    }

    uint64_t q_after3 = get_proc_dns_queries();
    if (q_after3 != q_after2 + 1) {
        pmsg("[dns-test] FAIL: expected 1 outbound query for NXDOMAIN probe\n");
        return 12;
    }
    pmsg("[dns-test] PASS: NXDOMAIN for nonexistent domain\n");

    freeaddrinfo(res);
    pmsg("[dns-test] ALL TESTS PASSED\n");
    return 0;
}
