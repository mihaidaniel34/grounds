class A {
    void m(String s, char[] cs) {
        String a = new String("x"); // expect
        String b = new String(); // expect
        String c = new String(s); // expect
        String ok = new String(cs);
    }
}
