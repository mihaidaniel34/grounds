class A {
    void m(int x) {
        // int y = x * 2; // expect
        // return y;
        // This explains why we do things; it is prose.
        int z = x; // trailing explanation with = sign;
        /* if (x > 0) { // expect
           x--;
        } */
    }
}
