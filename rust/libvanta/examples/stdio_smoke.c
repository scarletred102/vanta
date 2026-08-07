#include "vanta.h"

static const uint8_t path[] = "/home/vanta/stdio-smoke";
static const uint8_t payload[] = "stdio on Vanta\n";

int main(void) {
    vanta_file_t file;

    if (vanta_file_open(path, sizeof(path) - 1,
                        VANTA_OPEN_WRITE | VANTA_OPEN_CREATE |
                            VANTA_OPEN_TRUNCATE,
                        &file) < 0) {
        return 1;
    }
    if (vanta_file_putc(&file, payload[0]) != payload[0] ||
        vanta_file_write(&file, payload + 1, sizeof(payload) - 2) !=
            (int64_t)(sizeof(payload) - 2)) {
        vanta_file_close(&file);
        return 2;
    }
    if (vanta_file_flush(&file) < 0 ||
        vanta_file_putc(&file, '!') != '!' ||
        vanta_file_close(&file) < 0) {
        return 3;
    }

    if (vanta_file_open(path, sizeof(path) - 1, VANTA_OPEN_READ, &file) < 0) {
        return 4;
    }
    for (size_t index = 0; index < sizeof(payload) - 1; index++) {
        if (vanta_file_getc(&file) != payload[index]) {
            return 6;
        }
    }
    if (vanta_file_getc(&file) != '!' || vanta_file_getc(&file) != 0 ||
        vanta_file_close(&file) < 0) {
        return 5;
    }
    if (vanta_unlink(path, sizeof(path) - 1) < 0) {
        return 7;
    }

    static const uint8_t success[] = "libvanta stdio smoke passed\n";
    return vanta_write(1, success, sizeof(success) - 1) < 0 ? 8 : 0;
}
