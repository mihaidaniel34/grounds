class A {
    int v;
    @Override public boolean equals(Object o) {
        A other = (A) o; // expect
        return other.v == v;
    }
    @Override public int hashCode() { return v; }
}
class B {
    int v;
    @Override public boolean equals(Object o) {
        if (!(o instanceof B)) return false;
        return ((B) o).v == v;
    }
    @Override public int hashCode() { return v; }
}
