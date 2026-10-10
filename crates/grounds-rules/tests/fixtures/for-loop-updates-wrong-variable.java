class A {
    void m(int n) {
        for (int i = 0; i < n; i++) {
            for (int j = 0; j < n; i++) { } // expect
        }
        for (int k = 0; k < n; k++) { }
    }
}
