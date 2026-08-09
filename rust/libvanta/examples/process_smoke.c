#include "vanta.h"

static const uint8_t true_path[] = "/bin/true";
static const uint8_t false_path[] = "/bin/false";
static const uint8_t true_arg[] = "true";
static const uint8_t env_value[] = "VANTA_SPAWN=1";

int main(void) {
    const uint8_t *argv[] = {true_arg};
    const uint8_t *envp[] = {env_value};
    vanta_spawn_options_t options = {
        .stdin_fd = (uint64_t)-1,
        .stdout_fd = (uint64_t)-1,
        .stderr_fd = (uint64_t)-1,
        .argv = argv,
        .argc = 1,
        .envp = envp,
        .envc = 1,
    };
    int64_t true_pid = vanta_spawn_with_env(true_path, sizeof(true_path) - 1, &options);
    if (true_pid < 0 || vanta_waitpid((uint64_t)true_pid) != 0) {
        return 1;
    }
    int64_t false_pid = vanta_spawn(false_path, sizeof(false_path) - 1);
    if (false_pid < 0 || vanta_waitpid((uint64_t)false_pid) == 0) {
        return 2;
    }
    static const uint8_t message[] = "libvanta process smoke passed\n";
    return vanta_write(1, message, sizeof(message) - 1) < 0 ? 3 : 0;
}
