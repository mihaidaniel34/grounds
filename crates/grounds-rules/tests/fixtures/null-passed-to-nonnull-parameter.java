import org.jetbrains.annotations.NotNull;
class A {
    void take(@NotNull String s, String t) {}
    void m() {
        take(null, "x"); // expect
        take("x", null);
    }
}
class RejectsNull {
    void take(@NonNull String s) {}
    void tests() {
        org.junit.jupiter.api.Assertions.assertThrows(NullPointerException.class, () -> take(null));
        org.assertj.core.api.Assertions.assertThatThrownBy(() -> take(null)).isInstanceOf(NullPointerException.class);
        try {
            take(null);
            org.junit.jupiter.api.Assertions.fail("expected NPE");
        } catch (NullPointerException expected) {
        }
        take(null); // expect
    }
}
class RejectsNullEdges {
    Runnable saved;
    void take(@NonNull String s) {}
    String describe(@NonNull String s) { return s.trim(); }
    void tests() {
        try {
            take(null);
            org.junit.jupiter.api.Assertions.fail("expected NPE");
        } catch (NullPointerException expected) {
            take(null); // expect
        } finally {
            take(null); // expect
        }
        org.junit.jupiter.api.Assertions.assertThrows(IllegalStateException.class, () -> {
            saved = () -> take(null); // expect
            throw new IllegalStateException();
        });
        org.junit.jupiter.api.Assertions.assertThrows(IllegalArgumentException.class, () -> {}, () -> describe(null)); // expect
        org.junit.jupiter.api.Assertions.assertThrows(NullPointerException.class, (org.junit.jupiter.api.function.Executable) () -> take(null));
    }
}
class JUnit4MessageOverload {
    void take(@NonNull String s) {}
    void test() {
        org.junit.Assert.assertThrows("reject null", NullPointerException.class, () -> take(null));
        org.junit.Assert.assertThrows(NullPointerException.class, () -> take(null));
    }
}
