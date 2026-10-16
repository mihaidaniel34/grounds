class A {
    boolean m(Integer a, Integer b, int c, Long l1, Long l2, Boolean x, Boolean y) {
        if (a == b) return true; // expect
        if (l1 != l2) return true; // expect
        if (a == c) return true;
        if (a == null) return false;
        // the canonical instances: every Boolean but a `new Boolean(..)` is one of them
        if (x == Boolean.TRUE) return true;
        if (Boolean.FALSE != (y)) return true;
        if (x == java.lang.Boolean.TRUE) return true;
        return x == y; // expect
    }
}
class Shadowed {
    static class Boolean { static final Integer TRUE = 1000; }
    boolean m(Integer a) {
        return a == Boolean.TRUE; // expect
    }
}
