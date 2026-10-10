class A {
    private Object lock = new Object();
    private final Object fine = new Object();
    void m() {
        synchronized (lock) { } // expect
        synchronized (fine) { }
        synchronized (this) { }
    }
}
