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
class ViaHelper implements Cloneable {
    // the helper of the same class makes the copy with super.clone()
    @Override public Object clone() { try { return cloneReset(); } catch (CloneNotSupportedException e) { throw new AssertionError(e); } }
    Object cloneReset() throws CloneNotSupportedException { return super.clone(); }
}
class ViaOtherHelper implements Cloneable {
    @Override public Object clone() { return copy(); } // expect
    Object copy() { return new ViaOtherHelper(); }
}
class PlainPoint {
    int x;
    public PlainPoint clone() { PlainPoint p = new PlainPoint(); p.x = x; return p; }
}
