class A {
    void m(int x) throws Exception {
        if (x < 0) throw new RuntimeException("neg"); // expect
        if (x == 0) throw new Exception("zero"); // expect
        throw new IllegalArgumentException("x");
    }
}
