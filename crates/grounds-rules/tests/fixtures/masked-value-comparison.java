class A {
    boolean m(int x) {
        if ((x & 0xF0) == 0x0F) return true; // expect
        if ((x & 0) == 0) return true; // expect
        if ((x | 0x01) == 0x02) return true; // expect
        return (x & 0xF0) == 0x10;
    }
}
