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
class Callbacks {
    private static final org.slf4j.Logger log = org.slf4j.LoggerFactory.getLogger(Callbacks.class);
    void run(String name, missing.Client client) {
        client.call(name).doOnError(t -> log.error("failed for {} with cause", name, t));
    }
}
class UnknownTrailing {
    private static final org.slf4j.Logger log = org.slf4j.LoggerFactory.getLogger(UnknownTrailing.class);
    // the jar declaring these types is missing: the last argument may be the exception
    void m(String name, missing.MqttException mx, missing.Result result) {
        log.warn("connection lost for {}", name, mx);
        log.error("send failed for {} at {}", name, result.address(), result.cause());
        log.warn("no placeholder", name, mx); // expect
        log.warn("one {}", name, "two", mx); // expect
    }
}
