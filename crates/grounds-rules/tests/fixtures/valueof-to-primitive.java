class A {
    int m(String s) {
        int a = Integer.valueOf(s); // expect
        long b = Long.valueOf(s); // expect
        Integer c = Integer.valueOf(s);
        return Integer.parseInt(s);
    }
}
