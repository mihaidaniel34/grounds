class A {
    boolean m(double a, double b, float f, Double boxed) {
        if (a == b) return true; // expect
        if (f != 0.5f) return true; // expect
        if (a == 0.0) return false;
        if (a == 1) return false;
        if (boxed == a) return true; // expect
        if (a == Double.MAX_VALUE) return true;
        int i = 3;
        return i == 3;
    }
    public boolean equals(Object o) { return ((A) o).v == v; }
    double v;
}
