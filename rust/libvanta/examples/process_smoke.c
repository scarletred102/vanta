#include "vanta.h"

static const uint8_t true_path[] = "/bin/true";
static const uint8_t false_path[] = "/bin/false";

int main(void) {
    int64_t true_pid = vanta_spawn(true_path, sizeof(true_path) - 1);
    if (true_pid < 0 || vanta_waitpid((uint64_t)true_pid) != 0) {
        return 1;
    }
    int64_t false_pid = vanta_spawn(false_path, sizeof(false_path) - 1);
    if (false_pid < 0 || vanta_waitpid((uint64_t)false_pid) == 0) {
        return 2;
    }
    static const uint8_t message[] = "libvanta process smoke passed\n";
    return vanta_write(1, message, sizeof(message) - 1) < 0 ? 3 : 0;
}
