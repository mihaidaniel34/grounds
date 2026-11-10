// grounds: no-findings
import manifold.ext.rt.api.Structural;

// Manifold's javac plugin compiles this file: a structural interface is
// satisfied by any class with its methods, so `Impl` is assignable to `Shape`.
@Structural
interface Shape { String name(); }

class Impl { public String name() { return "x"; } }

class StructuralUse {
    static Impl make() { return new Impl(); }
    void test() { Shape x = make(); }
}
