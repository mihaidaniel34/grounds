class A {
    double m(int a, int b, long c) {
        double x = Math.ceil(a / b); // expect
        long y = Math.round(c); // expect
        double z = Math.ceil((double) a / b);
        return Math.floor((double) (a / b)); // expect
    }
}
