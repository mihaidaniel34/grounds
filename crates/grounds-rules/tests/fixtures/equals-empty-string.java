class A {
    boolean m(String s) {
        if (s.equals("")) return true; // expect
        return "".equals(s);
    }
}
