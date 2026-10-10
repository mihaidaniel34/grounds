class A {
    private static final ThreadLocal<String> CTX = new ThreadLocal<>(); // expect
    private static final ThreadLocal<String> OK = new ThreadLocal<>();
    void m() { CTX.set("x"); }
    void n() { OK.set("y"); try { } finally { OK.remove(); } }
}
