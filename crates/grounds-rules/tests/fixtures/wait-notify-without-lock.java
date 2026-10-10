class A {
    final Object lock = new Object();
    void m() throws InterruptedException {
        lock.notifyAll(); // expect
        synchronized (lock) { lock.notifyAll(); }
    }
    synchronized void n() { notify(); }
    void o() throws InterruptedException { wait(); } // expect
}
