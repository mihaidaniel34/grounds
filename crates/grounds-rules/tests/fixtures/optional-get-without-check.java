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
class OptionalAliases {
    String alias(Optional<String> o) {
        boolean has = o.isPresent();
        return has ? o.get() : "";
    }
    String shortCircuit(Optional<String> o) {
        final boolean hasBody = o.isPresent();
        if (hasBody && !o.get().isEmpty()) return o.get();
        return "";
    }
    String negated(Optional<String> o) {
        if (!o.isPresent() || o.get().isEmpty()) return "";
        return o.get();
    }
    String guava(Optional<String> o) {
        com.google.common.base.Preconditions.checkArgument(o.isPresent(), "missing");
        return o.get();
    }
    String hamcrest(Optional<String> o) {
        org.hamcrest.MatcherAssert.assertThat(o.isPresent(), org.hamcrest.Matchers.is(true));
        return o.get();
    }
    String assertjBoolean(Optional<String> o) {
        org.assertj.core.api.Assertions.assertThat(o.isPresent()).isTrue();
        return o.get();
    }
    java.util.List<String> filtered(java.util.List<Optional<String>> l) {
        l.stream().filter(Optional::isPresent).map(x -> x.get()).forEach(System.out::println);
        return l.stream().filter(opt -> opt.isPresent()).map(opt -> opt.get()).toList();
    }
}
class OptionalAliasHoles {
    static boolean isPresent(Optional<String> o) { return true; }
    java.util.List<String> otherFilter(java.util.List<Optional<String>> l) {
        return l.stream().filter(OptionalAliasHoles::isPresent).map(x -> x.get()).toList(); // expect
    }
    String reassigned(Optional<String> o, Optional<String> other) {
        boolean has = o.isPresent();
        o = other;
        return has ? o.get() : ""; // expect
    }
    String wrongBranch(Optional<String> o) {
        boolean has = o.isPresent();
        return has ? "" : o.get(); // expect
    }
    String hamcrestFalse(Optional<String> o) {
        org.hamcrest.MatcherAssert.assertThat(o.isPresent(), org.hamcrest.Matchers.is(false));
        return o.get(); // expect
    }
    java.util.List<String> otherFilter(java.util.List<Optional<String>> l) {
        return l.stream().filter(opt -> opt != null).map(opt -> opt.get()).toList(); // expect
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
