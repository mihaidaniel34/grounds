class A {
    void m() {
        synchronized (getClass()) { } // expect
        synchronized (A.class) { }
    }
}
