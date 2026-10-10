class A {
    int m(int a, int b) {
        int sum = a + b;
        return sum; // expect
    }
    int n(int a) {
        int x = a * 2;
        System.out.println(x);
        return x;
    }
}
