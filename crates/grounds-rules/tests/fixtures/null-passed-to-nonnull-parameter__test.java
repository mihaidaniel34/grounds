// grounds: test, grounds: no-findings
class ServiceTest {
    static class Service { void register(@javax.validation.constraints.NotNull String name) {} }
    Service service = new Service();
    void rejectsNullName() {
        // a test passing null on purpose, here to a bean-validation constraint
        service.register(null);
    }
}
