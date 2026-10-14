import java.util.function.*;
class Util {
    private static int twice(int x) { return 2 * x; }
    private int secret() { return 1; }
    static int name() { return 0; }
    void own() { twice(1); new Inner().peek(); }
    class Inner {
        int peek() { return twice(2) + secret(); }
    }
    static Runnable later() { return new Runnable() { public void run() { twice(3); } }; }
}
class Caller {
    void m(Util u) {
        Util.twice(1); // expect
        u.secret(); // expect
        Supplier<Integer> s = Util::name;
        IntSupplier t = Util::nmae; // expect
        Function<Util, Integer> f = Util::secret; // expect
        IntUnaryOperator tw = Util::twice; // expect
        Runnable r = u::own;
        Runnable q = u::gone; // expect
        Supplier<Util> mk = Util::new;
        Function<String, Integer> len = String::length;
    }
}
interface Api { default void run() {} }
class Base { private void run() {} private String field; }
class ViaDefault extends Base implements Api {
    void f(ViaDefault t) { t.run(); }
}
class Holder {
    Object field;
    class Inner extends Base {
        void f() { field.hashCode(); field.toString(); }
    }
}
// a superclass's private field hides the ones above it; the outer class's wins
class HOther {}
class HValue { void unique() {} }
class HBase { public HOther x; }
class HTest extends HBase {
    private HValue x;
    class Inner extends HTest {
        void f() { x.unique(); }
    }
}
// a superclass's private method is no member of the subclass (javac: cannot
// find symbol); an interface default of the same name is
class PBase { private void hidden() {} }
class PSub extends PBase { void f() { hidden(); } } // expect
class ViaDefault2 extends Base implements Api { void g() { run(); } }
