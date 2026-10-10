class A {
    int m(int x, double d) {
        int a = x % 1; // expect
        double frac = d % 1;
        return x % 2;
    }
}
