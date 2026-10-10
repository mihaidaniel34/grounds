class A {
    boolean m(Integer a, Integer b, int c, Long l1, Long l2, Boolean x, Boolean y) {
        if (a == b) return true; // expect
        if (l1 != l2) return true; // expect
        if (a == c) return true;
        if (a == null) return false;
        return x == y; // expect
    }
}
