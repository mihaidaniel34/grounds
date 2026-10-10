class A {
    void m() {
        try { work(); } catch (java.io.IOException e) { throw new IllegalStateException("failed"); } // expect
        try { work(); } catch (java.io.IOException e) { throw new IllegalStateException(e.getMessage()); } // expect
        try { work(); } catch (java.io.IOException e) { throw new IllegalStateException("failed", e); }
        try { work(); } catch (java.io.IOException e) { RuntimeException r = new RuntimeException("x"); r.initCause(e); throw r; }
        try { work(); } catch (java.io.IOException e) { throw new UncheckedIOException2(e); }
    }
    void work() throws java.io.IOException {}
}
class UncheckedIOException2 extends RuntimeException { UncheckedIOException2(Throwable t) { super(t); } }
