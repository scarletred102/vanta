int shared_counter = 10;

int calculate(int x) {
    return x * 2;
}

int secret_value(void) {
    return 1337;
}

int read_shared_counter(void) {
    return shared_counter;
}

void set_shared_counter(int val) {
    shared_counter = val;
}
