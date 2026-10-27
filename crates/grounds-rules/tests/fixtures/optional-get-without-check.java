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
class OptionalDerivedChecks {
    String mapOrElseFalse(Optional<String> observed, String desired) {
        if (observed.map(desired::equals).orElse(false)) {
            return observed.get();
        }
        return "";
    }
    String filterPresent(Optional<String> o) {
        if (o.filter(s -> !s.isEmpty()).isPresent()) return o.get();
        if (!o.map(String::trim).isEmpty()) return o.get();
        boolean usable = o.map(String::isEmpty).orElse(false);
        return usable ? o.get() : "";
    }
    String orElseTrue(Optional<String> o) {
        if (o.map(String::isEmpty).orElse(true)) {
            return o.get(); // expect
        }
        return "";
    }
    String orElseFalseNegated(Optional<String> o) {
        if (!o.map(String::isEmpty).orElse(false)) {
            return o.get(); // expect
        }
        return "";
    }
    String reassignedInChain(Optional<String> o) {
        if (o.map((o = Optional.<String>empty())::equals).isPresent()) return o.get(); // expect
        return "";
    }
    String mapOtherOptional(Optional<String> o, Optional<String> p) {
        if (p.map(String::isEmpty).orElse(false)) return o.get(); // expect
        return "";
    }
}
class OptionalCorrelatedFlag {
    String expected(boolean expected, Optional<String> o) {
        if (expected && o.isEmpty()) {
            throw new IllegalStateException("missing");
        }
        if (expected && !o.get().isEmpty()) return o.get();
        return "";
    }
    String negatedFlag(boolean lenient, Optional<String> o) {
        if (!lenient && !o.isPresent()) return "";
        if (!lenient) return o.get();
        return "";
    }
    String flagWrongValue(boolean expected, Optional<String> o) {
        if (expected && o.isEmpty()) throw new IllegalStateException();
        if (!expected) return o.get(); // expect
        return "";
    }
    String flagReassigned(boolean expected, Optional<String> o) {
        if (expected && o.isEmpty()) throw new IllegalStateException();
        expected = o.hashCode() > 0;
        if (expected) return o.get(); // expect
        return "";
    }
    String optionalReassigned(boolean expected, Optional<String> o, Optional<String> other) {
        if (expected && o.isEmpty()) throw new IllegalStateException();
        o = other;
        if (expected) return o.get(); // expect
        return "";
    }
    String noExit(boolean expected, Optional<String> o) {
        if (expected && o.isEmpty()) System.out.println("missing");
        if (expected) return o.get(); // expect
        return "";
    }
}
