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
// an inner class whose superclass we cannot see may hide the outer class's
// inherited `assertNotNull`
class Outer extends org.junit.Assert {
    class Inner extends ext.Base {
        void m(Object value) {
            assertNotNull(value);
            if (value == null) System.out.println("null");
        }
    }
    void n(Object value) {
        assertNotNull(value);
        if (value == null) System.out.println("null"); // expect
    }
}
// `assertTrue(v != null)` through a static import an unresolved superclass may hide
class ShadowedAssertTrue extends ext.Base {
    void m(Object value) {
        org.junit.Assert.assertTrue(value != null);
        if (value == null) System.out.println("null"); // expect
    }
}
