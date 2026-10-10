class A {
    boolean ready;
    synchronized void m() throws InterruptedException {
        if (!ready) wait(); // expect
        while (!ready) wait();
    }
}
