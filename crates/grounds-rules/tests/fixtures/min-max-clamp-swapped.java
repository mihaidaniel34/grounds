class A {
    int m(int x) {
        int a = Math.min(0, Math.max(100, x)); // expect
        int b = Math.max(100, Math.min(0, x)); // expect
        int ok = Math.min(100, Math.max(0, x));
        return Math.max(0, Math.min(100, x));
    }
}
