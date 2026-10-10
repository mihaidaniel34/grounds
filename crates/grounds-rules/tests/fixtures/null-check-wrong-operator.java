class A {
    boolean m(String s, String t) {
        if (s == null && s.isEmpty()) return true; // expect
        if (t != null || t.length() > 3) return true; // expect
        if (s != null && s.isEmpty()) return true;
        return t == null || t.isEmpty();
    }
}
