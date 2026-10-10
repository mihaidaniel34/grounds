class A {
    void m() {
        try { work(); } catch (Exception e) {
            if (e instanceof java.io.IOException) { } // expect
            if (e.getCause() instanceof RuntimeException) { }
        }
    }
    void work() throws Exception {}
}
