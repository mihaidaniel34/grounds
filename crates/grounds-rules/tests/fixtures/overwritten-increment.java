class A {
    int i;
    void m() {
        i = i++; // expect
        int j = 0;
        j = j--; // expect
        j = ++j;
        i++;
    }
}
