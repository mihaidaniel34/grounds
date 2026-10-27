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
    // stored for later: rethrown or reported after the catch
    private Throwable error;
    void retry() throws Exception {
        Throwable last = null;
        for (int i = 0; i < 3; i++) {
            try { work(); return; } catch (Throwable t) { last = t; }
        }
        throw new Exception("All repeated calls failed.", last);
    }
    void stores(Failure f, Handler h) {
        try { work(); } catch (Throwable t) { error = t; }
        try { work(); } catch (Throwable throwable) { this.error = throwable; }
        try { work(); } catch (Throwable t) { f.failure = t; }
        Throwable invokeException = null;
        try { work(); } catch (Throwable e) { invokeException = e; }
        h.handle(invokeException);
        try { work(); } catch (Throwable e) { Throwable stored = e; h.handle(stored); }
        try { work(); } catch (Error e) { log.warn("x", e); error = e; }
        // log-and-continue, or keeping only something derived from it
        String msg = null;
        int count = 0;
        try { work(); } catch (Throwable t) { log.warn("x", t); } // expect
        try { work(); } catch (Throwable t) { count++; } // expect
        try { work(); } catch (Throwable t) { msg = t.getMessage(); } // expect
        try { work(); } catch (Throwable t) { msg += t; } // expect
        try { work(); } catch (Throwable t) { String m = t.toString(); } // expect
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
class Failure { Throwable failure; }
interface Handler { void handle(Throwable t); }
