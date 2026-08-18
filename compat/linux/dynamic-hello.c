#include <unistd.h>
#include <string.h>

const char __interp[] __attribute__((section(".interp"))) = "/lib/ld-musl-x86_64.so.1";

int main(void) {
    const char msg1[] = "[linux-dynamic] dynamic interpreter loaded\n";
    const char msg2[] = "[linux-dynamic] hello from dynamic musl/glibc\n";
    write(1, msg1, sizeof(msg1) - 1);
    write(1, msg2, sizeof(msg2) - 1);
    return 0;
}
