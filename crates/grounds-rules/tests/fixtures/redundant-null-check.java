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
class NestedTwr {
    static Object make(Object f) throws Exception { return f; }
    static void close(Object o) {}
    static void f(Object factory) throws Exception {
        Object primary = null;
        try {
            try (AutoCloseable ctx = () -> {}) {
                primary = make(factory);
                primary.hashCode();
            }
        } finally {
            if (primary != null) close(primary);
        }
    }
}
