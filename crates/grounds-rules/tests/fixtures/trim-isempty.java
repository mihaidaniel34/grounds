class A {
    boolean m(String s) {
        if (s.trim().isEmpty()) return true; // expect
        return s.isBlank();
    }
}
