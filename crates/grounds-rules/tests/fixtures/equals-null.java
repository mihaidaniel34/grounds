class A {
    boolean m(String s, Object o) {
        if (s.equals(null)) return true; // expect
        return o != null && o.equals("x");
    }
    void contract(Object o) {
        org.junit.jupiter.api.Assertions.assertFalse(o.equals(null));
        org.junit.jupiter.api.Assertions.assertTrue(!o.equals(null));
    }
}
