#include <stdio.h>
#include <unistd.h>
#include <sys/stat.h>
#include <string.h>

int main(void) {
    // 1. Test access
    if (access("/etc/vanta-release", F_OK) != 0) {
        return 1;
    }

    // 2. Test stat
    struct stat st;
    if (stat("/etc/vanta-release", &st) != 0) {
        return 2;
    }
    if (st.st_size <= 0) {
        return 3;
    }

    // 3. Test fopen, fread, fseek
    FILE *fp = fopen("/etc/vanta-release", "r");
    if (!fp) {
        return 4;
    }
    char buf[128];
    memset(buf, 0, sizeof(buf));
    size_t n = fread(buf, 1, sizeof(buf) - 1, fp);
    if (n == 0) {
        fclose(fp);
        return 5;
    }
    if (fseek(fp, 0, SEEK_SET) != 0) {
        fclose(fp);
        return 6;
    }
    fclose(fp);

    static const char msg[] = "[linux-musl] file io passed\n";
    write(1, msg, sizeof(msg) - 1);
    return 0;
}
