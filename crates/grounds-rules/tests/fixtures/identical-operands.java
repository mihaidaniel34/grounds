class A {
    boolean m(int a, int b, double d, String s) {
        if (a == a) return true; // expect
        if (a > 0 && a > 0) return true; // expect
        int x = b - b; // expect
        boolean nan = d != d; // expect
        boolean e = s.equals(s); // expect
        int y = 1 << 1;
        if (a == b) return false;
        return m(1, 2, 3, "") == m(1, 2, 3, "");
    }
}
