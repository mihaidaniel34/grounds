class A {
    static final String K = "k";
    boolean m(String a, String b, Object o) {
        if (a == b) return true; // expect
        if (a != "x") return false; // expect
        if (a == null) return false;
        if (o == a) return true;
        if (K == "k") return true;
        return a.equals(b);
    }
}
