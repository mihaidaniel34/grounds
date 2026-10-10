class A {
    boolean m(int a, int b, double d) {
        if (!(a == b)) return true; // expect
        if (!(a < b)) return true; // expect
        if (!(d < 1.0)) return true;
        return !(a > b); // expect
    }
}
