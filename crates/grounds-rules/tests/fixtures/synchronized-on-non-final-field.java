class A {
    private Object lock = new Object();
    private final Object fine = new Object();
    private Object once;
    A() { once = new Object(); }
    void swap() { lock = new Object(); }
    void m() {
        // assigned in the constructor only: the monitor never changes
        synchronized (once) { }
        synchronized (lock) { } // expect
        synchronized (fine) { }
        synchronized (this) { }
    }
}
