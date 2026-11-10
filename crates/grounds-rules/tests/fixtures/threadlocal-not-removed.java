class A {
    private static final ThreadLocal<String> CTX = new ThreadLocal<>(); // expect
    private static final ThreadLocal<String> OK = new ThreadLocal<>();
    void m() { CTX.set("x"); }
    void n() { OK.set("y"); try { } finally { OK.remove(); } }
}
class Restored {
    private static final ThreadLocal<Boolean> HELD = new ThreadLocal<>();
    private static final ThreadLocal<String> CLEARED = new ThreadLocal<>();
    private static final ThreadLocal<String> ELSEWHERE = new ThreadLocal<>(); // expect
    private static final ThreadLocal<String> OTHER = new ThreadLocal<>();
    void covered(Runnable body) {
        Boolean previous = HELD.get();
        HELD.set(Boolean.TRUE);
        try { body.run(); } finally { HELD.set(previous); }
    }
    void scoped(Runnable body) {
        CLEARED.set("x");
        try { body.run(); } finally { CLEARED.set(null); }
    }
    // the finally cleans up a different ThreadLocal
    void leaky(Runnable body) {
        ELSEWHERE.set("x");
        try { body.run(); } finally { OTHER.remove(); }
    }
    // one method restores, another does not
    void alsoLeaky() { HELD.get(); ELSEWHERE.set("y"); }
}
