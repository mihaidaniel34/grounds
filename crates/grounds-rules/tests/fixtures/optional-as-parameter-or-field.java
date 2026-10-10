import java.util.Optional;
class A {
    private Optional<String> name; // expect
    void m(Optional<String> o) { } // expect
    Optional<String> ok() { return Optional.empty(); }
    void lambda() { java.util.function.Function<Optional<String>, String> f = (Optional<String> x) -> ""; }
}
