#include <unistd.h>
#include <sys/wait.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv) {
    if (argc > 1 && strcmp(argv[1], "--child") == 0) {
        return 42;
    }

    static const char msg[] = "[linux-musl] script sequencing passed\n";
    write(1, msg, sizeof(msg) - 1);
    return 0;
}
