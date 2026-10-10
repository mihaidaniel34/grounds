import org.slf4j.Logger;
import org.slf4j.LoggerFactory;
class A {
    private static final Logger log = LoggerFactory.getLogger(A.class);
    void m(String user) {
        log.debug("user " + user + " logged in"); // expect
        log.info(String.format("user %s", user)); // expect
        log.info("constant " + "concat");
        if (log.isDebugEnabled()) {
            log.debug("guarded " + user);
        }
        log.info("user {}", user);
    }
}
