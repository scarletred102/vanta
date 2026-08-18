#include <stdlib.h>
#include <string.h>
#include <unistd.h>

int main(void) {
    // 1. Test malloc & free
    char *buf1 = (char *)malloc(1024);
    if (!buf1) {
        return 1;
    }
    memset(buf1, 0x41, 1024);
    for (int i = 0; i < 1024; i++) {
        if (buf1[i] != 0x41) {
            free(buf1);
            return 2;
        }
    }
    free(buf1);

    // 2. Test calloc (zero initialized)
    int *buf2 = (int *)calloc(256, sizeof(int));
    if (!buf2) {
        return 3;
    }
    for (int i = 0; i < 256; i++) {
        if (buf2[i] != 0) {
            free(buf2);
            return 4;
        }
        buf2[i] = i * 7;
    }

    // 3. Test realloc (larger)
    int *buf3 = (int *)realloc(buf2, 1024 * sizeof(int));
    if (!buf3) {
        return 5;
    }
    for (int i = 0; i < 256; i++) {
        if (buf3[i] != i * 7) {
            free(buf3);
            return 6;
        }
    }

    // 4. Test larger allocation requiring mmap
    char *large = (char *)malloc(64 * 1024);
    if (!large) {
        free(buf3);
        return 7;
    }
    memset(large, 0x5a, 64 * 1024);
    if (large[64 * 1024 - 1] != 0x5a) {
        free(large);
        free(buf3);
        return 8;
    }

    free(large);
    free(buf3);

    static const char msg[] = "[linux-musl] memory allocation passed\n";
    write(1, msg, sizeof(msg) - 1);
    return 0;
}
