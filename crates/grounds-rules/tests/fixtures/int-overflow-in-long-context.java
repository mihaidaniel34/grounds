class A {
    long m(int days, int x) {
        long millis = days * 24 * 3600 * 1000; // expect
        long ok = days * 24L * 3600 * 1000;
        long small = x * 2;
        long big = x * x; // expect
        return days * 86_400_000; // expect
    }
}
