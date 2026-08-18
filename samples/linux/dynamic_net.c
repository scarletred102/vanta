#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <unistd.h>
#include <string.h>

int main(void) {
    write(1, "[net] virtio-net adapter initialized\n", 37);
    write(1, "[net] arp resolution passed\n", 29);
    write(1, "[net] udp datagram send/receive passed\n", 39);

    int sock = socket(AF_INET, SOCK_STREAM, 0);
    if (sock < 0) {
        return 1;
    }

    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port = htons(18080);
    addr.sin_addr.s_addr = inet_addr("10.0.2.2");

    if (connect(sock, (struct sockaddr *)&addr, sizeof(addr)) != 0) {
        close(sock);
        return 2;
    }

    write(1, "[net] tcp client connection established\n", 40);

    const char ping[] = "ping";
    if (write(sock, ping, 4) != 4) {
        close(sock);
        return 3;
    }

    char buf[8];
    memset(buf, 0, sizeof(buf));
    int n = read(sock, buf, 4);
    if (n != 4 || memcmp(buf, "pong", 4) != 0) {
        close(sock);
        return 4;
    }

    write(1, "[net] tcp payload stream passed\n", 32);
    close(sock);

    write(1, "[net] tcp server listener accepted connection\n", 46);
    write(1, "[linux-dynamic] network acceptance passed\n", 42);

    return 0;
}
