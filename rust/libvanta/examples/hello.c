#include "vanta.h"

int main(void) {
    vanta_abi_info_t info;
    if (vanta_get_abi_info(&info) < 0 || info.abi_version != 0 ||
        info.struct_size < sizeof(info)) {
        return 2;
    }
    static const uint8_t message[] = "hello from C on Vanta\n";
    return vanta_write(1, message, sizeof(message) - 1) < 0 ? 1 : 0;
}
