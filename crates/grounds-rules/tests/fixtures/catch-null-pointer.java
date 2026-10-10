class A {
    void m(String s) {
        try { s.length(); } catch (NullPointerException e) { } // expect
        try { s.length(); } catch (ClassCastException e) { } // expect
        try { s.length(); } catch (IllegalStateException e) { }
    }
}
