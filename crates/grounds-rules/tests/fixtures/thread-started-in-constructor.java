class A {
    private final Thread worker;
    A() {
        worker = new Thread(() -> {});
        worker.start(); // expect
    }
    void begin() { worker.start(); }
}
