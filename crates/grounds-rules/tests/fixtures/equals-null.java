class A {
    boolean m(String s, Object o) {
        if (s.equals(null)) return true; // expect
        return o != null && o.equals("x");
    }
}
