import org.springframework.transaction.annotation.Transactional;
// Spring Boot proxies classes by default: a final method is not intercepted
interface Service { int value(); }
class Implementation implements Service {
    @Transactional
    public final int value() { return 1; } // expect
}
