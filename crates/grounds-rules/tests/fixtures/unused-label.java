class A {
    void m(int[][] a) {
        outer: // expect
        for (int[] row : a) { }
        rows:
        for (int[] row : a) {
            for (int v : row) { if (v == 0) continue rows; }
        }
    }
}
