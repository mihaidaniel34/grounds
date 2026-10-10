class A {
    int m(String s, boolean c) {
        if (s == null) {
            return s.length(); // expect
        }
        String t = null;
        if (c) t = "x";
        int n = t.length();
        String u = null;
        u.trim(); // expect
        String v = c ? null : "y";
        v.isEmpty();
        String w = s;
        if (w != null) { w.trim(); }
        return n;
    }
    void n(@javax.annotation.Nullable String p) {
        p.trim(); // expect
    }
    void o(String q) {
        if (q != null && q.isEmpty()) return;
        if (q == null) return;
        q.trim();
    }
}
