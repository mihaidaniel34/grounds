class A {
    int m(int x) {
        int a = compute(); // expect
        a = 5;
        int b = 0;
        b = x;
        int c = x * 2;
        c = c + 1;
        String s = null;
        s = "y";
        int d = 1;
        if (x > 0) d = 2;
        return a + b + c + d + s.length();
    }
    void n() {
        int e = compute();
        Runnable r = () -> System.out.println(e);
    }
    int compute() { return 1; }
}
