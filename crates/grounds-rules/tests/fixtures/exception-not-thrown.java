class A {
    void m(int x) {
        if (x < 0) {
            new IllegalArgumentException("negative"); // expect
        }
        new StringBuilder("x");
        throw new IllegalStateException();
    }
}
