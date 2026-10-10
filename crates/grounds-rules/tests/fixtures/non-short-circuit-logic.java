class A {
    boolean m(String s, boolean a, boolean b) {
        if (s != null & s.isEmpty()) return true; // expect
        if (a | check()) return true; // expect
        boolean c = a & b;
        return a && check();
    }
    boolean check() { return true; }
}
