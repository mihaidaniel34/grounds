class A {
    void m(int x, boolean b) {
        int y = 0;
        y =+ x; // expect
        y =- x; // expect
        b =! b; // expect
        y = -x;
        y = +x;
        y += x;
    }
}
