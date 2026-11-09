import java.util.List;
import java.util.function.UnaryOperator;
class Node {
    @interface Nullable { }
    @interface Contract { String value(); }
    @Nullable Node getParent() { return null; }
    @Nullable Object getValue() { return null; }
    String name() { return ""; }

    @Contract("null, _ -> null; !null, _ -> !null")
    static <T> @Nullable List<T> map(@Nullable List<T> ls, UnaryOperator<T> f) { return ls; }
    @Nullable static List<String> lookup() { return null; }

    // the same getter was just compared with null
    Node root(Node cursor) {
        while (cursor.getParent() != null) {
            cursor = cursor.getParent();
        }
        return cursor;
    }
    String checkedGetter(Node n) {
        if (n.getParent() == null) {
            return "";
        }
        Node p = n.getParent();
        return p.name();
    }
    String castAfterInstanceof(Node n) {
        if (n.getValue() instanceof Node) {
            Node v = (Node) n.getValue();
            return v.name();
        }
        return "";
    }
    String reassignedReceiver(Node n, Node other) {
        if (n.getParent() != null) {
            n = other;
            Node p = n.getParent();
            return p.name(); // expect
        }
        return "";
    }
    String unchecked(Node n) {
        Node p = n.getParent();
        return p.name(); // expect
    }

    // @Contract: null only for a null argument
    int contract(List<String> in) {
        List<String> out = map(in, s -> s);
        return out.size();
    }
    int contractNullable(@Nullable List<String> in) {
        List<String> out = map(in, s -> s);
        return out.size(); // expect
    }
    int noContract() {
        List<String> out = lookup();
        return out.size(); // expect
    }

    // null is passed along; the result is what gets checked
    String propagated(Node n) {
        String name = n == null ? null : n.name();
        if (name == null) {
            return "";
        }
        return n.name();
    }
    String checkedThenUsed(Node n) {
        if (n == null) {
            System.out.println("none");
        }
        return n.name(); // expect
    }

    // a project predicate given the value usually checks it
    static boolean usable(@Nullable Node n) { return n != null && !n.name().isEmpty(); }
    String guardedByPredicate(@Nullable Node n) {
        if (usable(n)) {
            return n.name();
        }
        return "";
    }
    String notGuarded(@Nullable Node n) {
        return n.name(); // expect
    }
}
