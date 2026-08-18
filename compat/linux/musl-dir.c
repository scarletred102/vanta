#include <dirent.h>
#include <unistd.h>
#include <string.h>

int main(void) {
    DIR *d = opendir("/etc");
    if (!d) {
        return 1;
    }
    int found_release = 0;
    struct dirent *ent;
    while ((ent = readdir(d)) != NULL) {
        if (strcmp(ent->d_name, "vanta-release") == 0) {
            found_release = 1;
        }
    }
    closedir(d);

    if (!found_release) {
        return 2;
    }

    static const char msg[] = "[linux-musl] directory iteration passed\n";
    write(1, msg, sizeof(msg) - 1);
    return 0;
}
