class A {
    private static int count;
    private static final Object LOCK = new Object();
    void inc() {
        count++; // expect
        count = 5; // expect
    }
    static void reset() { count = 0; }
    synchronized void safe() { count = 1; }
    A() { }
}
