class Base {
    void run() {}
    String name() { return ""; }
}
class Sub extends Base {
    void run() {} // expect
    @Override String name() { return "x"; }
    public String toString() { return ""; } // expect
    void own() {}
}
