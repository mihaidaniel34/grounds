// grounds: test
import static org.junit.jupiter.api.Assertions.*;
class ATest {
    void t() {
        assertThrows(IllegalStateException.class, () -> { prepare(); run(); }); // expect
        assertThrows(IllegalStateException.class, () -> run());
        assertThrows(IllegalStateException.class, () -> run(build()));
        assertThrows(IllegalStateException.class, () -> { Object o = build(); run(o); }); // expect
        assertThrows(IllegalStateException.class, () -> prepare() + compute()); // expect
    }
    void chains(Builder b) {
        // a builder chain is one invocation
        assertThrows(IllegalStateException.class, () -> b.name("x").size(build()).build());
        assertThrows(IllegalStateException.class, () -> new Builder().name("x").build());
        // a comment is not a statement
        assertThrows(IllegalStateException.class, () -> {
            // only build() validates
            b.name("x").build();
        });
        assertThrows(IllegalStateException.class, () -> { // expect
            b.name("x").build();
            b.build();
        });
    }
    void deferred(Visitor v) {
        // the anonymous class body and nested lambdas do not run with the lambda
        assertThrows(IllegalStateException.class, () -> new Visitor() {
            @Override public void visit(Object o) { prepare(); run(o); }
        }.visit(build()));
        assertThrows(IllegalStateException.class, () -> v.accept(o -> { prepare(); run(o); }));
        // a catch clause runs only after the failure
        assertThrows(IllegalStateException.class, () -> {
            try {
                run();
            } catch (RuntimeException e) {
                prepare();
                throw e;
            }
        });
        // fail() only reports that nothing was thrown
        assertThrows(IllegalStateException.class, () -> {
            run();
            fail("not thrown");
        });
    }
    int prepare() { return 0; }
    int compute() { return 0; }
    void run() {}
    void run(Object o) {}
    Object build() { return null; }
    static class Builder { Builder name(String n) { return this; } Builder size(Object s) { return this; } Object build() { return null; } }
    static class Visitor { public void visit(Object o) {} void accept(java.util.function.Consumer<Object> c) {} }
}
