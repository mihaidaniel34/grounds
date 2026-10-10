import org.jetbrains.annotations.NotNull;
class A {
    @NotNull String m(int x) {
        if (x > 0) return "a";
        return null; // expect
    }
    String n() { return null; }
}
