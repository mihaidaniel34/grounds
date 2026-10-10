class A {
    boolean m(String s) {
        if (s.equals("x")) return true; // expect
        if (s.equalsIgnoreCase("Y")) return true; // expect
        if ("x".equals(s)) return true;
        return s.equals(s);
    }
}
