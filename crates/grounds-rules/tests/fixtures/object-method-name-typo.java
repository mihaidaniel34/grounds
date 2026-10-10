class A {
    public int hashcode() { return 1; } // expect
    public String tostring() { return ""; } // expect
    public boolean equal(Object o) { return false; } // expect
    public int hashCode() { return 2; }
}
