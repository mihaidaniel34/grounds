class A {
    private int m(int used, int notUsed) { // expect
        return used;
    }
    public int api(int x) { return 0; }
    private void stub(int x) { throw new UnsupportedOperationException(); }
    private static int s(int a, int b) { return a; } // expect
    void call() { m(1, 2); s(1, 2); }
}
