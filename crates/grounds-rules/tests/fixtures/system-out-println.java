class A {
    void m(String s) {
        System.out.println(s); // expect
        System.err.printf("%s", s); // expect
    }
    public static void main(String[] args) { System.out.println("usage"); }
}
