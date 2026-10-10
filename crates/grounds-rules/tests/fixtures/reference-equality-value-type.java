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
// identity as a no-change shortcut in immutable types
final class Node {
    final Money price;
    Node(Money price) { this.price = price; }
    @Override public boolean equals(Object o) { return o instanceof Node; }
    @Override public int hashCode() { return 1; }
    Node withPrice(Money price) { return this.price == price ? this : new Node(price); }
    Node visit(Node n, Object ctx) {
        if (n == ctx || !(ctx instanceof Node)) {
            return n;
        }
        return (Node) ctx;
    }
    Node keep(Money price) {
        if (this.price != price) return this; // expect
        return new Node(price);
    }
    Node changed(Node a, Node b) {
        if (a != b) return a;
        return b;
    }
    boolean same(Node a, Node b) {
        return a == b ? a.price != null : false; // expect
    }
}
