class A {
    void m() {
        while (true) { // expect
            work();
        }
    }
    void n() {
        for (;;) { // expect
            work();
        }
    }
    int o() {
        while (true) {
            if (done()) return 1;
        }
    }
    void p() {
        while (true) {
            if (done()) break;
        }
    }
    void q() {
        while (true) {
            try { Thread.sleep(1); } catch (InterruptedException e) { return; }
        }
    }
    void work() {}
    boolean done() { return true; }
}
