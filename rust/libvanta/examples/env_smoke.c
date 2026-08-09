#include "vanta.h"

int main(void) {
    static const uint8_t variable[] = "VANTA_ABI_VERSION";
    const uint8_t *value = vanta_getenv(variable, sizeof(variable) - 1);
    const uint8_t *const *environment = vanta_environ();
    if (environment == 0 || environment[0] == 0 || value == 0 ||
        value[0] != '0' || value[1] != 0) {
        return 1;
    }
    static const uint8_t message[] = "libvanta environment smoke passed\n";
    return vanta_write(1, message, sizeof(message) - 1) < 0 ? 2 : 0;
}
