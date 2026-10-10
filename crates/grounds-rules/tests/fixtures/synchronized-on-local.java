class A {
    void m() {
        synchronized (new Object()) { } // expect
        Object lock = new Object();
        synchronized (lock) { } // expect
        synchronized (this) { }
    }
}
