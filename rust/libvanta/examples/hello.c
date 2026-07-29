#include "vanta.h"

int main(void) {
    static const uint8_t message[] = "hello from C on Vanta\n";
    return vanta_write(1, message, sizeof(message) - 1) < 0 ? 1 : 0;
}
