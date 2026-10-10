class A {
    static final String WILDCARD = "*";
    static final String NOTHING = null;
    boolean n() { return new String("ok").equals(NOTHING); }
    boolean c(String s) {
        if (s.equals(WILDCARD)) return true; // expect
        return WILDCARD.equals(s);
    }
    boolean m(String s) {
        if (s.equals("x")) return true; // expect
        if (s.equalsIgnoreCase("Y")) return true; // expect
        if ("x".equals(s)) return true;
        return s.equals(s);
    }
}
