import org.slf4j.Logger;
import org.slf4j.LoggerFactory;
class A {
    private static final Logger log = LoggerFactory.getLogger(A.class);
    void m() {
        try { work(); } catch (Exception e) { log.error("failed: " + e.getMessage()); } // expect
        try { work(); } catch (Exception e) { log.error("failed: {}", e.getMessage()); } // expect
        try { work(); } catch (Exception e) { log.error("failed: {}", e.getMessage(), e); }
        try { work(); } catch (Exception e) { log.debug("failed: " + e.getMessage()); }
    }
    void work() throws Exception {}
}
