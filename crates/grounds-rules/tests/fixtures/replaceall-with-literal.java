class A {
    String m(String s) {
        String a = s.replaceAll("abc", "x"); // expect
        String b = s.replaceAll("a.c", "x");
        String c = s.replaceAll("\\s+", " ");
        String d = s.replaceAll("x", "$1");
        return s.replaceAll(",", ";"); // expect
    }
}
