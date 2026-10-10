class A {
    String m(String s) {
        String a = String.format("hello"); // expect
        String b = String.format("100%%");
        return String.format("%s", s);
    }
}
