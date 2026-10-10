class A {
    void m() {
        System.gc(); // expect
        Runtime.getRuntime().gc(); // expect
    }
}
