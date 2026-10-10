class A {
    void m() {
        System.exit(1); // expect
        Runtime.getRuntime().halt(2); // expect
    }
    public static void main(String[] args) { System.exit(0); }
}
