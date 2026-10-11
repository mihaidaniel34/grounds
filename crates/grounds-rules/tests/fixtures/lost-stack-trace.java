class A {
    void m() {
        try { work(); } catch (java.io.IOException e) { throw new IllegalStateException("failed"); } // expect
        try { work(); } catch (java.io.IOException e) { throw new IllegalStateException(e.getMessage()); } // expect
        try { work(); } catch (java.io.IOException e) { throw new IllegalStateException("failed", e); }
        try { work(); } catch (java.io.IOException e) { RuntimeException r = new RuntimeException("x"); r.initCause(e); throw r; }
        try { work(); } catch (java.io.IOException e) { throw new UncheckedIOException2(e); }
        try { work(); } catch (java.io.IOException e) { Throwable cause = e.getCause(); throw new UncheckedIOException2(cause); }
        try { work(); } catch (java.io.IOException e) { throw new UncheckedIOException2(e.getCause()); }
        try { work(); } catch (java.io.IOException e) { throw new IllegalStateException("failed: " + e.getClass().getSimpleName()); } // expect
        try { work(); } catch (java.io.IOException e) { handle(e); throw new IllegalStateException("unreachable"); }
        try { work(); } catch (java.io.IOException e) { if (isMissing(e)) { throw new IllegalStateException("missing"); } } // expect
        try { work(); } catch (java.io.IOException e) { isMissing(e); throw new IllegalStateException("failed"); } // expect
        try { work(); } catch (java.io.IOException e) { if (debug) { handle(e); throw new IllegalStateException("failed"); } }
        try { work(); } catch (java.io.IOException e) { missing.Lib.rethrowUnchecked(e); throw new IllegalStateException("unreachable"); }
        try { work(); } catch (java.io.IOException e) { if (e.getCause() instanceof java.net.SocketException se) { throw new IllegalStateException("socket", se); } }
        try { work(); } catch (java.io.IOException e) { if ((Object) String.valueOf(e) instanceof String msg) { throw new IllegalStateException(msg); } } // expect
        try { work(); } catch (java.io.IOException e) { throw new IllegalStateException("failed"); } finally { handle(null); } // expect
        try { work(); } catch (java.io.IOException e) { if (debug) handle(e); throw new IllegalStateException("failed"); } // expect
        try { work(); } catch (java.io.IOException e) { String msg = String.valueOf(e); throw new IllegalStateException(msg); } // expect
        try { work(); } catch (missing.RemoteException e) { throw new IllegalStateException(e.getMessage()); } // expect
    }
    void work() throws java.io.IOException {}
    void handle(Exception e) { throw new RuntimeException(e); }
    boolean isMissing(Exception e) { return false; }
    boolean debug;
}
class UncheckedIOException2 extends RuntimeException { UncheckedIOException2(Throwable t) { super(t); } }
class LocalStore {
    void m() {
        try { work(); } catch (java.io.IOException e) { Throwable last = null; last = e; throw new RuntimeException("failed"); } // expect
    }
    void work() throws java.io.IOException {}
}
class AliasCleared {
    void m() {
        Throwable last = null;
        try { work(); } catch (java.io.IOException e) { last = e; last = null; throw new RuntimeException("failed", last); } // expect
    }
    void work() throws java.io.IOException {}
}
