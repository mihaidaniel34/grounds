class A {
    boolean m(boolean b, int i) {
        int j = ~~i; // expect
        return !!b; // expect
    }
}
