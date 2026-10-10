class A {
    Boolean m(int x) {
        if (x > 0) return true;
        return null; // expect
    }
    @javax.annotation.Nullable Boolean n() { return null; }
    boolean p() { return false; }
}
