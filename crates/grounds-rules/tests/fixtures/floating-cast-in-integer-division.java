class A {
    int m(double d) {
        int a = (int) Math.random() * 10; // expect
        int b = (int) (Math.random() * 10);
        int c = (int) d * 2;
        return a + b + c;
    }
}
