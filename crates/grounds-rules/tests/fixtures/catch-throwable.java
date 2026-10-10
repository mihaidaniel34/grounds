import org.slf4j.Logger;
import org.slf4j.LoggerFactory;
class A {
    private static final Logger log = LoggerFactory.getLogger(A.class);
    void m(java.util.concurrent.CompletableFuture<String> f) {
        try { work(); } catch (Throwable t) { log.error("x", t); } // expect
        try { work(); } catch (Error e) { System.out.println(e); } // expect
        try { work(); } catch (Throwable t) { cleanup(); throw t; }
        try { work(); } catch (Throwable t) { f.completeExceptionally(t); }
        try { work(); } catch (Throwable t) { throw new IllegalStateException(t); }
        try { work(); } catch (Exception e) { log.error("x", e); }
    }
    public static void main(String[] args) {
        try { } catch (Throwable t) { }
    }
    void work() {}
    void cleanup() {}
}
