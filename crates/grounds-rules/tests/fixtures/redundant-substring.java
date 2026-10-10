class A {
    String m(String s, int i) {
        String a = s.substring(0); // expect
        String b = s.substring(i, s.length()); // expect
        return s.substring(i);
    }
}
