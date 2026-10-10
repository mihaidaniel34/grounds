class A {
    void m(int n) {
        for (int i = 0; i < n; i--) { } // expect
        for (int j = n; j > 0; j++) { } // expect
        for (int k = 0; k < n; k += 2) { }
        for (int l = n; l >= 0; l--) { }
        for (int m = 0; m < n; m -= 1) { } // expect
    }
}
