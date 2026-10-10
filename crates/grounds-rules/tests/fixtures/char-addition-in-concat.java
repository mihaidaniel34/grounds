class A {
    String m(char a, char b, String s) {
        String x = a + b + ""; // expect
        String ok = "" + a + b;
        String ok2 = s + a + b;
        int code = a + b;
        return x;
    }
}
