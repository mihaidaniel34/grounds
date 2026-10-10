class A {
    private void neverCalled() { } // expect
    private void called() { }
    private void viaRef() { }
    private void overloaded(int x) { } // expect
    private void overloaded(String s) { }
    private void writeObject(java.io.ObjectOutputStream o) { }
    void m() {
        called();
        Runnable r = this::viaRef;
        overloaded("x");
    }
}
