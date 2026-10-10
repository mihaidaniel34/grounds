class A {
    int m(int x) {
        if (x > 0) {
            return 1;
        } else { // expect
            x++;
        }
        if (x > 1) { x--; } else { x++; }
        return x;
    }
}
