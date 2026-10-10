class A {
    void m(int i, long l, double d, short s, float f) {
        i += d; // expect
        i *= l; // expect
        s += 1;
        l += i;
        d += i;
        f += d; // expect
        i += 2;
    }
}
