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
class OptionalGuards {
    void assertj(java.util.Optional<String> o) {
        org.assertj.core.api.Assertions.assertThat(o).isPresent();
        o.get();
    }
    void junit(java.util.Optional<String> o) {
        org.junit.jupiter.api.Assertions.assertTrue(o.isPresent());
        o.get();
    }
    void lambda(java.util.Optional<String> maybe) {
        java.util.Optional<String> opt = maybe;
        if (!opt.isPresent()) return;
        Runnable r = () -> System.out.println(opt.get());
    }
}
class OptionalHoles {
    void soft(org.assertj.core.api.SoftAssertions softly, java.util.Optional<String> opt) {
        softly.assertThat(opt).isPresent();
        opt.get(); // expect
    }
    void logged(java.util.Optional<String> opt) {
        System.out.println(opt.isPresent());
        Runnable r = () -> System.out.println(opt.get()); // expect
        r.run();
    }
}
