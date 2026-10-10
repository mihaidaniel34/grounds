class A {
    int m(int x) {
        int unusedButNamed = 1;
        int a = compute(); // expect
        int b = 2; // expect
        b = 3;
        int c = 4;
        for (int i = 0; i < x; i++) { }
        try (java.io.StringReader r = new java.io.StringReader("")) { } catch (Exception e) { }
        return c;
    }
    int compute() { return 1; }
}
