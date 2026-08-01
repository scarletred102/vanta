#include "vanta.h"

static const uint8_t home[] = "/home/vanta";
static const uint8_t smoke_dir[] = "/tmp/c-sdk-smoke";

int main(void) {
    vanta_abi_info_t abi;
    vanta_stat_t stat;
    vanta_pipe_t pipe;
    uint8_t entries[256];

    if (vanta_get_abi_info(&abi) < 0 || abi.abi_version != 0 ||
        abi.struct_size < sizeof(abi)) {
        return 1;
    }
    if (vanta_getpid() <= 0 || vanta_getppid() < 0) {
        return 2;
    }

    int64_t directory = vanta_open(home, sizeof(home) - 1, 0x10);
    if (directory < 0 || vanta_fstat((uint64_t)directory, &stat) < 0 ||
        vanta_getdents((uint64_t)directory, entries, sizeof(entries)) < 0) {
        return 3;
    }
    int64_t duplicate = vanta_dup((uint64_t)directory);
    if (duplicate < 0) {
        return 4;
    }
    vanta_close((uint64_t)duplicate);
    vanta_close((uint64_t)directory);

    if (vanta_mkdir(smoke_dir, sizeof(smoke_dir) - 1) < 0 ||
        vanta_unlink(smoke_dir, sizeof(smoke_dir) - 1) < 0) {
        return 5;
    }
    if (vanta_pipe(&pipe) < 0) {
        return 6;
    }
    vanta_close(pipe.read_fd);
    vanta_close(pipe.write_fd);
    vanta_yield();

    static const uint8_t message[] = "libvanta SDK smoke passed\n";
    return vanta_write(1, message, sizeof(message) - 1) < 0 ? 7 : 0;
}
