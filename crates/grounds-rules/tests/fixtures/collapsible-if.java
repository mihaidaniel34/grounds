class A {
    void m(boolean a, boolean b) {
        if (a) { // expect
            if (b) {
                work();
            }
        }
        if (a) {
            if (b) { work(); }
            work();
        }
        if (a) { if (b) { work(); } else { work(); } }
    }
    void work() {}
}
