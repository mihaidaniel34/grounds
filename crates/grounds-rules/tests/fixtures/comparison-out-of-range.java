class A {
    boolean m(byte b, char c, short s) {
        if (b == 200) return true; // expect
        if (b > 127) return true; // expect
        if (c < -1) return true; // expect
        if (s == 40000) return true; // expect
        if ((b & 0xFF) == 200) return true;
        return b == -1 || c == 'x';
    }
}
