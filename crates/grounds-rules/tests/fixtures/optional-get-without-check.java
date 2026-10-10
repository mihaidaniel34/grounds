import java.util.Optional;
class A {
    String m(Optional<String> o, Optional<String> p, Optional<String> q) {
        String a = o.get(); // expect
        if (p.isPresent()) {
            return p.get();
        }
        if (q.isEmpty()) return "";
        String b = q.get();
        Optional<String> r = Optional.of("x");
        return a + b + r.get();
    }
}
