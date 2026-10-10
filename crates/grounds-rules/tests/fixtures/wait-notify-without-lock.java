class A {
    final Object lock = new Object();
    public void m() throws InterruptedException {
        lock.notifyAll(); // expect
        synchronized (lock) { lock.notifyAll(); }
    }
    synchronized void n() { notify(); }
    public void o() throws InterruptedException { wait(); } // expect
    // helpers inherit the caller's lock
    private void signal() { notifyAll(); }
    public synchronized void publish() { signal(); }
    public void wake() { notifyAll(); }
    public void caller() { synchronized (this) { wake(); } }
}
