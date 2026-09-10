typedef unsigned long size_t;

extern int calculate(int x);
extern int secret_value(void);

static long sys_write(int fd, const void *buf, size_t count) {
    long ret;
    __asm__ volatile ("syscall" : "=a"(ret) : "a"(1), "D"(fd), "S"(buf), "d"(count) : "rcx", "r11", "memory");
    return ret;
}

static void sys_exit(int code) {
    __asm__ volatile ("syscall" : : "a"(60), "D"(code) : "rcx", "r11", "memory");
    for (;;) {}
}

static void print_str(const char *s) {
    size_t len = 0;
    while (s[len]) len++;
    sys_write(1, s, len);
}

static void print_num(int n) {
    char buf[16];
    int i = 0;
    if (n == 0) {
        sys_write(1, "0", 1);
        return;
    }
    while (n > 0) {
        buf[i++] = '0' + (n % 10);
        n /= 10;
    }
    for (int j = 0; j < i / 2; j++) {
        char tmp = buf[j];
        buf[j] = buf[i - 1 - j];
        buf[i - 1 - j] = tmp;
    }
    sys_write(1, buf, i);
}

int main(void) {
    print_str("[dynamic-shlib] main entered\n");
    int r = calculate(42);
    print_str("[dynamic-shlib] calculate(42) = ");
    print_num(r);
    print_str("\n");

    int s = secret_value();
    print_str("[dynamic-shlib] secret_value() = ");
    print_num(s);
    print_str("\n");

    if (r == 84 && s == 1337) {
        print_str("[dynamic-shlib] SUCCESS: cross-boundary call to libcalc.so verified (84, 1337)\n");
        return 0;
    } else {
        print_str("[dynamic-shlib] FAILURE: unexpected return values\n");
        return 1;
    }
}

void _start(void) {
    int ret = main();
    sys_exit(ret);
}
