class Base {
    void run(int x) {}
    protected void hook() {}
}
class Sub extends Base {
    @Override void run(int x) { super.run(x); } // expect
    @Override public void hook() { super.hook(); }
}
class Sub2 extends Base {
    @Override void run(int x) { super.run(x + 1); }
}
