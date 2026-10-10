class A {
    synchronized void m() throws InterruptedException {
        Thread.sleep(100); // expect
    }
    void n() throws InterruptedException {
        synchronized (this) { Thread.sleep(1); } // expect
        Thread.sleep(1);
    }
}
