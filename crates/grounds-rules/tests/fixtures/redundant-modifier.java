interface I {
    public void a(); // expect
    abstract void b(); // expect
    public static final int X = 1; // expect
    void c();
}
class A {
    static interface Nested { } // expect
    private final void p() { } // expect
}
