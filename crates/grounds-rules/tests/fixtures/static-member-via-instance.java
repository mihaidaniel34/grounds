class A {
    static int counter;
    static void util() {}
    void m(A other) {
        other.util(); // expect
        int c = other.counter; // expect
        A.util();
        util();
    }
}
