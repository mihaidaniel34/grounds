import java.util.Optional;
class A {
    Optional<String> find(Optional<String> o) {
        if (o == null) return Optional.empty(); // expect
        Optional<String> x = null; // expect
        x = null; // expect
        return null; // expect
    }
    String other() { return null; }
}
