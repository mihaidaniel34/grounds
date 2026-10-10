class A {
    synchronized void m() {
        notify(); // expect
        notifyAll();
    }
}
