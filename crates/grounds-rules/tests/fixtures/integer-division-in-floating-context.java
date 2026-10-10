class A {
    double ratio(int done, int total, long l) {
        double r = done / total; // expect
        float f = done / 3; // expect
        double ok = (double) done / total;
        double c = (double) (done / total); // expect
        double d = 1.5 * (done / total); // expect
        int i = done / total;
        double half = 4 / 2;
        return done / total; // expect
    }
}
