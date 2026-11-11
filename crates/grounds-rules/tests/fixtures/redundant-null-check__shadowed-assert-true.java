import static org.junit.Assert.assertTrue;

// `assertTrue` imported statically in a class whose superclass we cannot
// see: that superclass may declare its own `assertTrue`, which does not throw
class Hidden extends ext.Base {
    void m(Object value) {
        assertTrue(value != null);
        if (value == null) System.out.println("null");
    }
}
class Plain {
    void m(Object value) {
        assertTrue(value != null);
        if (value == null) System.out.println("null"); // expect
    }
}
