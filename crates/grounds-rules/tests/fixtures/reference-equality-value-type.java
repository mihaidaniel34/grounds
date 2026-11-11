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
    // a library value too: true exactly when round() gave back v itself
    java.math.BigDecimal round(java.math.BigDecimal v) { return v; }
    boolean library(java.math.BigDecimal v) {
        java.math.BigDecimal r = round(v);
        return r == v;
    }
    // a helper that always makes a new object: never the same one
    static java.math.BigDecimal copy(java.math.BigDecimal v) { return new java.math.BigDecimal(v.toString()); }
    boolean sameValue(java.math.BigDecimal v) {
        java.math.BigDecimal r = copy(v);
        return r == v; // expect
    }
    boolean sameValueInline(java.math.BigDecimal v) {
        return copy(v) == v; // expect
    }
    // a local replaced after it started out as the other operand
    boolean reassigned(java.math.BigDecimal v) {
        java.math.BigDecimal r = v;
        r = new java.math.BigDecimal(v.toString());
        return r == v; // expect
    }
    // a local that switches between two objects: which one it holds
    boolean active(java.util.ArrayList<String> first, java.util.ArrayList<String> rest, boolean more) {
        var elements = first;
        if (more) elements = rest;
        return elements == first;
    }
    boolean transformed(java.math.BigDecimal v) {
        java.math.BigDecimal r = round(v);
        r = r.stripTrailingZeros();
        return r == v;
    }
    // an element is not the collection it came from
    java.util.List<java.math.BigDecimal> all(java.math.BigDecimal v) { return java.util.List.of(v); }
    boolean element(java.math.BigDecimal v) {
        for (java.math.BigDecimal x : all(v)) {
            if (x == v) return true; // expect
        }
        return false;
    }
    // two locals of one name in separate blocks: each is its own
    boolean scoped(java.math.BigDecimal v, boolean c) {
        if (c) {
            java.math.BigDecimal r = new java.math.BigDecimal("1");
            if (r.signum() > 0) return false;
        }
        java.math.BigDecimal r = round(v);
        return r == v;
    }
    // a helper that may hand its argument back stays silent
    java.math.BigDecimal maybe(java.math.BigDecimal v) { return v.signum() == 0 ? v : new java.math.BigDecimal(v.toString()); }
    boolean unknown(java.math.BigDecimal v) {
        java.math.BigDecimal r = maybe(v);
        return r == v;
    }
    // but not against another value
    boolean libraryUnrelated(java.math.BigDecimal v, java.math.BigDecimal w) {
        java.math.BigDecimal r = round(v);
        return r == w; // expect
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
