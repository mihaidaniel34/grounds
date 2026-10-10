class A {
    static final int YEAR_MS = 365 * 24 * 60 * 60 * 1000; // expect
    static final long OK = 365L * 24 * 60 * 60 * 1000;
    static final int FINE = 24 * 60 * 60;
    int m() { return Integer.MAX_VALUE + 1; } // expect
    static final int SIGN_BIT = 1 << 31;
    static final int MASK = 0xFFFFFFFF;
}
