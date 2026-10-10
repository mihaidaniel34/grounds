class A {
    boolean m(double d, float f) {
        if (d == Double.NaN) return true; // expect
        if (f != Float.NaN) return true; // expect
        return Double.isNaN(d);
    }
}
