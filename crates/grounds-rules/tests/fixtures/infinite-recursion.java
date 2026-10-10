class A {
    private String name;
    String getName() { return getName(); } // expect
    int fact(int n) { return n <= 1 ? 1 : n * fact(n - 1); }
    void loop(int x) { // expect
        System.out.println(x);
        loop(x + 1);
    }
    void delegate(A other) { other.delegate(this); }
    void guarded(int x) { if (x > 0) guarded(x - 1); }
}
