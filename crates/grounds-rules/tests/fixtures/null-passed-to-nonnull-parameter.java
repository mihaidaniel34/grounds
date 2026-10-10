import org.jetbrains.annotations.NotNull;
class A {
    void take(@NotNull String s, String t) {}
    void m() {
        take(null, "x"); // expect
        take("x", null);
    }
}
