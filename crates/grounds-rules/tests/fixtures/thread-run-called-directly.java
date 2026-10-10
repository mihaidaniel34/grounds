class A {
    void m(Thread t, Runnable r) {
        t.run(); // expect
        r.run();
        t.start();
    }
}
class B extends Thread { public void run() { super.run(); } }
