class A {
    void m() {
        try { work(); } catch (Exception e) { } // expect
        try { work(); } catch (Exception ignored) { }
        try { work(); } catch (Exception e) {
            // safe: best effort
        }
        try { work(); } catch (RuntimeException e) { log(e); }
    }
    void work() {}
    void log(Exception e) {}
}
