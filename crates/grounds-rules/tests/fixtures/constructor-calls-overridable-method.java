class A {
    A() {
        init(); // expect
        this.setup(); // expect
        helper();
        fin();
    }
    void init() {}
    protected void setup() {}
    private void helper() {}
    final void fin() {}
}
final class B {
    B() { init(); }
    void init() {}
}
class C {
    private C() { init(); }
    void init() {}
}
