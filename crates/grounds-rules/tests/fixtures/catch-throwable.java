import org.slf4j.Logger;
import org.slf4j.LoggerFactory;
import org.nolib.Sink;
import org.nolib.Subscriber;
import org.nolib.Results;
class A {
    private static final Logger log = LoggerFactory.getLogger(A.class);
    void m(java.util.concurrent.CompletableFuture<String> f) {
        try { work(); } catch (Throwable t) { log.error("x", t); } // expect
        try { work(); } catch (Error e) { System.out.println(e); } // expect
        try { work(); } catch (Throwable t) { cleanup(); throw t; }
        try { work(); } catch (Throwable t) { f.completeExceptionally(t); }
        try { work(); } catch (Throwable t) { throw new IllegalStateException(t); }
        try { work(); } catch (Exception e) { log.error("x", e); }
        try { work(); } catch (Throwable t) { Tracer.logError("x", t); } // expect
        // logger held in a holder class, its type unknown (library jar missing)
        try { work(); } catch (Throwable t) { Holders.MAIN.error("shutdown failed", t); } // expect
        try { work(); } catch (Throwable t) { Holders.MAIN.warn("shutdown failed", t); } // expect
    }
    Flow n(Subscriber s) {
        try { work(); } catch (Throwable e) { return Flow.error(e); }
        try { work(); } catch (Throwable e) { return Results.error("x", e); }
        try { work(); } catch (Throwable e) { return org.nolib.Results.error("x", e); }
        try { work(); } catch (Throwable t) { org.nolib.Loggers.MAIN.warn("x", t); } // expect
        try { work(); } catch (Throwable e) { s.onError(e); }
        try { work(); } catch (Throwable e) { s.error(e); }
        try { work(); } catch (Throwable t) { throw rethrow(t); }
        try { work(); } catch (Throwable e) { throw new IllegalStateException("x", e); }
        try { work(); } catch (Throwable e) { submitForReprocessing("msg", e); }
        return null;
    }
    public static void main(String[] args) {
        try { } catch (Throwable t) { }
    }
    void work() {}
    void cleanup() {}
    RuntimeException rethrow(Throwable t) { return new RuntimeException(t); }
    void submitForReprocessing(String msg, Throwable e) {}
}
class Tracer { static void logError(String m, Throwable t) {} }
class Holders { static final Sink MAIN = null; }
class Flow { static Flow error(Throwable t) { return new Flow(); } }
