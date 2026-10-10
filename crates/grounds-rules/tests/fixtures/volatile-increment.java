class A {
    private volatile int count;
    private int plain;
    void m() {
        count++; // expect
        count += 2; // expect
        plain++;
        count = 5;
    }
    synchronized void n() { count++; }
}
