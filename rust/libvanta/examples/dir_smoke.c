#include "vanta.h"

static const uint8_t bin[] = "/bin";

int main(void) {
    vanta_dir_t directory;
    char name[64];
    void *allocation = vanta_malloc(32);
    if (allocation == 0) {
        return 1;
    }
    vanta_free(allocation);
    if (vanta_dir_open(bin, sizeof(bin) - 1, &directory) < 0) {
        return 2;
    }
    if (vanta_dir_read(&directory, name, sizeof(name)) <= 0) {
        return 3;
    }
    vanta_dir_close(&directory);
    static const uint8_t message[] = "libvanta directory smoke passed\n";
    return vanta_write(1, message, sizeof(message) - 1) < 0 ? 5 : 0;
}
