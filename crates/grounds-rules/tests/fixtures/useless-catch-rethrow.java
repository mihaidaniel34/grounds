class A {
    void m() throws Exception {
        try { work(); } catch (Exception e) { throw e; } // expect
        try { work(); } catch (java.io.IOException e) { throw e; } catch (Exception e) { log(e); }
        try { work(); } catch (Exception e) { log(e); throw e; }
    }
    void work() throws java.io.IOException {}
    void log(Exception e) {}
}
