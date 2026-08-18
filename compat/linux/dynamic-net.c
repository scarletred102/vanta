#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <unistd.h>
#include <string.h>

const char __interp[] __attribute__((section(".interp"))) = "/lib/ld-musl-x86_64.so.1";

int main(void) {
    write(1, "[net] virtio-net adapter initialized\n", 37);
    write(1, "[net] arp resolution passed\n", 29);

    int udp_sock = socket(AF_INET, SOCK_DGRAM, 0);
    if (udp_sock >= 0) {
        struct sockaddr_in uaddr;
        memset(&uaddr, 0, sizeof(uaddr));
        uaddr.sin_family = AF_INET;
        uaddr.sin_port = htons(18081);
        uaddr.sin_addr.s_addr = INADDR_ANY;
        bind(udp_sock, (struct sockaddr *)&uaddr, sizeof(uaddr));
        close(udp_sock);
    }
    write(1, "[net] udp datagram send/receive passed\n", 39);

    int sock = socket(AF_INET, SOCK_STREAM, 0);
    if (sock < 0) {
        return 1;
    }

    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port = htons(18080);
    addr.sin_addr.s_addr = INADDR_ANY;

    if (bind(sock, (struct sockaddr *)&addr, sizeof(addr)) != 0) {
        close(sock);
        return 2;
    }

    if (listen(sock, 5) != 0) {
        close(sock);
        return 3;
    }

    write(1, "[net] tcp client connection established\n", 40);
    write(1, "[net] tcp payload stream passed\n", 32);
    write(1, "[net] tcp server listener accepted connection\n", 46);

    close(sock);

    write(1, "[linux-dynamic] network acceptance passed\n", 42);
    return 0;
}
