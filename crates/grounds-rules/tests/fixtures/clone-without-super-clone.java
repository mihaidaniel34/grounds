class A implements Cloneable {
    int x;
    @Override public A clone() { A a = new A(); a.x = x; return a; } // expect
}
class B implements Cloneable {
    @Override public B clone() throws CloneNotSupportedException { return (B) super.clone(); }
}
final class C implements Cloneable {
    @Override public C clone() { return new C(); }
}
class PlainPoint {
    int x;
    public PlainPoint clone() { PlainPoint p = new PlainPoint(); p.x = x; return p; }
}
