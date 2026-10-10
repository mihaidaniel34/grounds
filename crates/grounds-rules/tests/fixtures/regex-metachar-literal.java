class A {
    void m(String s) {
        String[] a = s.split("."); // expect
        String b = s.replaceAll("|", "-"); // expect
        String[] ok = s.split("\\.");
        String[] ok2 = s.split(",");
    }
}
