import org.slf4j.Logger;
import org.slf4j.LoggerFactory;
class A {
    private static final Logger log = LoggerFactory.getLogger(A.class);
    void m() throws Exception {
        try { work(); } catch (Exception e) { log.error("failed", e); throw e; } // expect
        try { work(); } catch (Exception e) { log.error("failed", e); }
        try { work(); } catch (Exception e) { throw e; }
    }
    void work() throws Exception {}
}
