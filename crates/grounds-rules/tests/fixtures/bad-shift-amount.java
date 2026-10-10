class A {
    long m(int i, long l) {
        int a = i << 32; // expect
        long b = l >> 64; // expect
        int c = i >>> -1; // expect
        long ok = l << 40;
        i <<= 33; // expect
        return (long) i << 40;
    }
}
