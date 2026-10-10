class A {
    @Override protected void finalize() throws Throwable { super.finalize(); } // expect
    void m(A other) throws Throwable { other.finalize(); } // expect
}
