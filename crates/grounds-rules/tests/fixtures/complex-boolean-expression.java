class A {
    boolean m(boolean a, boolean b, boolean c, boolean d, boolean e) {
        if (a && b || c && d || e) return true; // expect
        return a && b;
    }
}
