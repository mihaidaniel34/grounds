import org.slf4j.Logger;
import org.slf4j.LoggerFactory;
class A {
    private static final Logger log = LoggerFactory.getLogger(A.class);
    void m(String user, int id, Exception e) {
        log.info("user {} id {}", user); // expect
        log.debug("user {}", user, id); // expect
        log.warn("no placeholder", user); // expect
        log.error("failed for {}", user, e);
        log.error("failed", e);
        log.info("user {} id {}", user, id);
        log.info("literal \\{} {}", user);
    }
}
