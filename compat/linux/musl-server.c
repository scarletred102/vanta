#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <string.h>

int main(void) {
    int s = socket(AF_INET, SOCK_STREAM, 0);
    if (s < 0) {
        return 1;
    }

    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port = 8080;
    addr.sin_addr.s_addr = 0;

    // Test bind, listen, or close
    bind(s, (struct sockaddr *)&addr, sizeof(addr));
    listen(s, 5);
    close(s);

    static const char msg[] = "[linux-musl] socket execution passed\n";
    write(1, msg, sizeof(msg) - 1);
    return 0;
}
