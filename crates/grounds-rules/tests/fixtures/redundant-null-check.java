class A {
    void m(String s) {
        s.trim();
        if (s == null) return; // expect
        String t = new String("x");
        if (t != null) { } // expect
        String u = null;
        if (u == null) { } // expect
    }
    void n(String p) {
        if (p != null) { }
        if (p == null) return;
        assert p != null;
    }
}
