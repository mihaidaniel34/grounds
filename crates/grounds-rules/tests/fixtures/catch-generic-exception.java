class A {
    void m() {
        try { } catch (Exception e) { } // expect
        try { } catch (IllegalStateException e) { }
    }
}
