class Outer {
    int field;
    class Uses {
        int get() { return field; }
    }
    class Independent { // expect
        int x;
        int get() { return x * 2; }
    }
    class CallsOuter {
        void run() { work(); }
    }
    static class AlreadyStatic { }
    void work() {}
}
class OuterTest {
    @org.junit.jupiter.api.Nested
    class InnerTests { int x; }
}
class Generic<P> {
    // names the outer type parameter: cannot be static
    class Visitor { P visit(P p) { return p; } }
    class Plain { int x; } // expect
}
