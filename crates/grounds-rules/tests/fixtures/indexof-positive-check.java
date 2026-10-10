class A {
    boolean m(String s) {
        if (s.indexOf("x") > 0) return true; // expect
        if (0 < s.indexOf('y')) return true; // expect
        if (s.indexOf("x", 1) > 0) return true;
        return s.indexOf("z") >= 0;
    }
}
