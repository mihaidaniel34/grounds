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
class Sentinels {
    static class Slot { volatile java.util.Optional<String> resolved; }
    void record(Slot slot) {
        java.util.Optional<String> r = slot == null ? java.util.Optional.empty() : slot.resolved;
        if (r != null && r.isPresent()) System.out.println(r.get());
    }
    void cache(java.util.Map<String, java.util.Optional<String>> cache, String k) {
        if (cache.get(k) == null) { }
    }
}
