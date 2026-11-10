import java.util.AbstractList;
import java.util.Set;

// Only members take part in a call: a private method of a superclass is not
// inherited (JLS 8.2), a static method of an interface is not inherited
// (JLS 8.4.8), and an inaccessible method is not applicable (JLS 15.12.2.1).
class SitesBase {
    public static class A { }
    private static A sites() { return new A(); }
}

class SitesOuter {
    public static class B { public int digest; }
    private static B sites() { return new B(); }

    public static class Inner extends SitesBase {
        int f() { return sites().digest; }
        int g() { return sites().hash(); } // expect
    }
}

abstract class Tup extends AbstractList<Object> {
    static Tup of(Object... elems) { return null; }
}

class TupSet {
    void update(Tup others) { }
}

class UseTup {
    void f(TupSet x, Set<?> s) { x.update(Tup.of(s)); }
    void g(TupSet x, Set<?> s) { x.update(Tup.of(s).subList(0, 1)); } // expect
}

class Seg { }
class ShortSeg extends Seg { }
class Proj { }
class ShortProj extends Proj { }
class Found { Proj get() { return null; } }

class Engine {
    public Found create(Seg s) { return new Found(); }
    private ShortProj create(ShortSeg s) { return new ShortProj(); }
    ShortProj own() { return create(new ShortSeg()); }
}

class UseEngine {
    Proj f() { return new Engine().create(new ShortSeg()).get(); }
    Proj g() { return new Engine().create(new ShortSeg()).take(); } // expect
}
