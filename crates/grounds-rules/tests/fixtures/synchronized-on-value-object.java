class A {
    private final Integer count = 0;
    private final Object lock = new Object();
    private static final String KEY = "k";
    void m() {
        synchronized ("lock") { } // expect
        synchronized (count) { } // expect
        synchronized (KEY) { } // expect
        synchronized (lock) { }
    }
}
