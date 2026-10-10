class Money {
    long cents;
    @Override public boolean equals(Object o) { return o instanceof Money && ((Money) o).cents == cents && this == o; }
    @Override public int hashCode() { return 1; }
}
class Plain {}
enum E { X }
class A {
    boolean m(Money a, Money b, Plain p, Plain q, E e, E f) {
        if (a == b) return true; // expect
        if (a != null) return false;
        if (p == q) return true;
        return e == f;
    }
}
