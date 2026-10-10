class A {
    String m(String s, Object o) {
        String a = s.toString(); // expect
        String b = o.toString();
        return "x".toString(); // expect
    }
}
