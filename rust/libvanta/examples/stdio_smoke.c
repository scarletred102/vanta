#include "vanta.h"

static const uint8_t path[] = "/home/vanta/stdio-smoke";
static const uint8_t payload[] = "stdio on Vanta\n";

int main(void) {
    vanta_stream_t stream;
    uint8_t buffer[sizeof(payload)];

    if (vanta_stream_open(path, sizeof(path) - 1,
                          VANTA_OPEN_WRITE | VANTA_OPEN_CREATE |
                              VANTA_OPEN_TRUNCATE,
                          &stream) < 0) {
        return 1;
    }
    if (vanta_stream_write(&stream, payload, sizeof(payload) - 1) !=
        (int64_t)(sizeof(payload) - 1)) {
        vanta_stream_close(&stream);
        return 2;
    }
    if (vanta_stream_flush(&stream) < 0 || vanta_stream_close(&stream) < 0) {
        return 3;
    }

    if (vanta_stream_open(path, sizeof(path) - 1, VANTA_OPEN_READ, &stream) < 0) {
        return 4;
    }
    int64_t count = vanta_stream_read(&stream, buffer, sizeof(buffer));
    if (count != (int64_t)(sizeof(payload) - 1) ||
        vanta_stream_close(&stream) < 0) {
        return 5;
    }
    for (size_t index = 0; index < sizeof(payload) - 1; index++) {
        if (buffer[index] != payload[index]) {
            return 6;
        }
    }
    if (vanta_unlink(path, sizeof(path) - 1) < 0) {
        return 7;
    }

    static const uint8_t success[] = "libvanta stdio smoke passed\n";
    return vanta_write(1, success, sizeof(success) - 1) < 0 ? 8 : 0;
}
