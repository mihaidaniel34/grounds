class A {
    void m() {
        try { work(); } catch (Exception e) { e.printStackTrace(); } // expect
        try { work(); } catch (Exception e) { e.printStackTrace(System.out); }
    }
    void work() throws Exception {}
}
