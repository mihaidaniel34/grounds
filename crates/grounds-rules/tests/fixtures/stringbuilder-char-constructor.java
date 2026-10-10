class A {
    void m(char c) {
        StringBuilder a = new StringBuilder('a'); // expect
        StringBuffer b = new StringBuffer(c); // expect
        StringBuilder ok = new StringBuilder("a");
        StringBuilder ok2 = new StringBuilder(16);
    }
}
