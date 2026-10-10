class A {
    int v;
    public boolean equals(A other) { return other.v == v; } // expect
}
class B {
    public boolean equals(B other) { return true; }
    public boolean equals(Object o) { return o instanceof B && equals((B) o); }
    public int hashCode() { return 0; }
}
