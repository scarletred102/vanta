typedef unsigned long size_t;

extern int calculate(int x);
extern int secret_value(void);
extern int shared_counter;
extern int read_shared_counter(void);
extern void set_shared_counter(int val);

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

    if (r != 84 || s != 1337) {
        print_str("[dynamic-shlib] FAILURE: unexpected function return values\n");
        return 1;
    }
    print_str("[dynamic-shlib] SUCCESS: cross-boundary call to libcalc.so verified (84, 1337)\n");

    print_str("[dynamic-shlib] --- Testing cross-boundary data relocation ---\n");
    int c_before = shared_counter;
    print_str("[dynamic-shlib] initial shared_counter read via main GOT: ");
    print_num(c_before);
    print_str("\n");

    // Main executable mutates shared_counter via GOT
    shared_counter += 32; // 10 + 32 = 42
    int c_main_after = shared_counter;
    print_str("[dynamic-shlib] after main write (+=32), shared_counter: ");
    print_num(c_main_after);
    print_str("\n");

    // Shared library reads the variable from within libcalc.so
    int c_so_sees = read_shared_counter();
    print_str("[dynamic-shlib] libcalc.so read_shared_counter() sees: ");
    print_num(c_so_sees);
    print_str("\n");

    // Shared library mutates the variable
    set_shared_counter(100);
    int c_main_sees_lib = shared_counter;
    print_str("[dynamic-shlib] after libcalc set_shared_counter(100), main sees: ");
    print_num(c_main_sees_lib);
    print_str("\n");

    // Another direct write from main
    shared_counter = 777;
    int c_so_sees2 = read_shared_counter();
    print_str("[dynamic-shlib] after main write (777), libcalc sees: ");
    print_num(c_so_sees2);
    print_str("\n");

    if (c_before == 10 && c_main_after == 42 && c_so_sees == 42 && c_main_sees_lib == 100 && c_so_sees2 == 777) {
        print_str("[dynamic-shlib] SUCCESS: cross-boundary data relocation verified\n");
        print_str("[dynamic-shlib] before=10 after_write=42 so_sees=42 second=777\n");
        return 0;
    } else {
        print_str("[dynamic-shlib] FAILURE: cross-boundary data relocation mismatch\n");
        return 2;
    }
}

void _start(void) {
    int ret = main();
    sys_exit(ret);
}
