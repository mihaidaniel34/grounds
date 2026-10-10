class A {
    static final boolean DEBUG = false;
    void m(boolean x) {
        if (true) { work(); } // expect
        if (DEBUG) { work(); }
        boolean y = x || true; // expect
        boolean z = false && x; // expect
        int v = false ? 1 : 2; // expect
    }
    void work() {}
}
