class A {
    void m(int[] a) {
        for (int i = 0; i < a.length; i++) {
            if (a[i] == 0) i++; // expect
        }
        for (int j = 0; j < a.length; j++) {
            int k = j + 1;
        }
    }
}
