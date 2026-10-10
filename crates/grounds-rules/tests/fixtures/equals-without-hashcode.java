class A {
    int v;
    @Override public boolean equals(Object o) { return o instanceof A && ((A) o).v == v; } // expect
}
class B {
    @Override public int hashCode() { return 1; } // expect
}
class C {
    public boolean equals(Object o) { return false; }
    public int hashCode() { return 0; }
}
@lombok.EqualsAndHashCode
class D { int x; }
class Token {
    @Override public boolean equals(Object o) { return o == this; }
}
class NotIdentity {
    int id;
    @Override public boolean equals(Object o) { return this.id == ((NotIdentity) o).id; } // expect
}
class Everything {
    @Override public boolean equals(Object other) { return this == this; } // expect
}
