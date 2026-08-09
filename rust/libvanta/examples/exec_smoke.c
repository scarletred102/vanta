#include "vanta.h"

static const uint8_t true_path[] = "/bin/true";

int main(void) {
    vanta_exec(true_path, sizeof(true_path) - 1);
    return 1;
}
