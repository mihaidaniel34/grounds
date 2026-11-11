import static java.util.Objects.requireNonNull;

// a superclass that is not on the classpath may declare its own
// requireNonNull, which hides the imported one: not a contract
class ShadowedStatic extends ext.Base {
    void m(Object value) {
        requireNonNull(value);
        if (value == null) System.out.println("null");
    }
}
class NotShadowed {
    void m(Object value) {
        requireNonNull(value);
        if (value == null) System.out.println("null"); // expect
    }
}
