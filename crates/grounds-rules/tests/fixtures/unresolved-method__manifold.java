// grounds: no-findings
import manifold.ext.rt.api.Jailbreak;

// Manifold's javac plugin compiles this file (its runtime API is imported):
// `@Jailbreak` gives access to private members, extension classes add
// methods to any class. Valid with the plugin, so nothing is reported.
class ManifoldBase {
    private int foo(int x) { return x; }
}

class ManifoldCase {
    int f() {
        @Jailbreak ManifoldBase b = new ManifoldBase();
        return b.foo(1);
    }
    int g(ManifoldBase b) { return b.bar(); }
}
