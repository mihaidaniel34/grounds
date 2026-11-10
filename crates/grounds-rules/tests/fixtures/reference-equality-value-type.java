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
// identity as "the transformation gave back what it was given"
class Rewriter {
    Money visit(Money m, Object ctx) { return m; }
    Money price() { return null; }
    boolean unchanged(Money before, Object ctx) {
        Money after = visit(before, ctx);
        return after == before;
    }
    boolean inline(Money m, Object ctx) { return m != visit(m, ctx); }
    boolean getter(Object ctx) {
        Money p = visit(price(), ctx);
        return p == price();
    }
    boolean accumulated(Money original, Object ctx) {
        Money m = original;
        for (int i = 0; i < 3; i++) m = visit(m, ctx);
        return m != original;
    }
    // a library value: an equal result is a new object
    java.math.BigDecimal round(java.math.BigDecimal v) { return v; }
    boolean library(java.math.BigDecimal v) {
        java.math.BigDecimal r = round(v);
        return r == v; // expect
    }
    // unrelated values: identity is not a change check
    boolean unrelated(Money a, Money b, Object ctx) {
        Money after = visit(a, ctx);
        return after == b; // expect
    }
    boolean fresh(Money a) {
        Money b = price();
        return a == b; // expect
    }
}
