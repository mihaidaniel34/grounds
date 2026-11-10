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
class UnknownArguments {
    private com.unknown.Codec codec;
    public void go(com.unknown.Code rc) { close(rc); }
    // the argument types come from a missing jar: either overload may be the target
    private void close(com.unknown.Code code) { close(codec.encode(code)); }
    private void close(byte code) { close(codec.message(code)); }
    private void close(com.unknown.Message msg) { }
}
