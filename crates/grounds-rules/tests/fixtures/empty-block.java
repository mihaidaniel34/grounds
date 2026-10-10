class A {
    void m(int x) {
        if (x > 0) { } // expect
        if (x > 1) {
            // intentionally nothing
        }
        try { work(); } finally { } // expect
        synchronized (this) { } // expect
        for (int i = 0; i < x; i++) { } // expect
        if (x > 2) { work(); } else { } // expect
    }
    void work() {}
}
