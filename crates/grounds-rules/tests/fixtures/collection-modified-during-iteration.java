import java.util.*;
class A {
    void m(HashMap<String, String> m) {
        List<String> l = new ArrayList<>(List.of("a", ""));
        for (String s : l) {
            if (s.isEmpty()) l.remove(s); // expect
        }
        for (String k : m.keySet()) {
            m.remove(k); // expect
        }
        l.forEach(s -> l.add(s)); // expect
        for (String s : l) {
            if (s.isEmpty()) { l.remove(s); break; }
        }
        for (String s : new ArrayList<>(l)) { l.remove(s); }
        l.removeIf(String::isEmpty);
        for (String x : l) {
            inner: for (int i = 0; i < 1; i++) {
                l.remove(x); // expect
                continue inner;
            }
        }
        scan: for (String x : l) {
            l.remove(x); // expect
            continue scan;
        }
        List<String> incoming = new ArrayList<>(List.of("b"));
        next: for (String e : l) {
            for (String in : incoming) {
                if (in.equals(e)) { incoming.remove(in); continue next; }
            }
        }
    }
    // declared by interface: nearly always a plain list, reported as a warning
    void param(List<String> l) {
        for (String s : l) { l.remove(s); } // expect
        List<String> cow = new java.util.concurrent.CopyOnWriteArrayList<>(List.of("x"));
        for (String s : cow) { cow.remove(s); }
    }
}
class OfUnseenOrigin {
    private final Map<String, String> shared = new java.util.concurrent.ConcurrentHashMap<>();
    private final Map<String, String> plain = new HashMap<>();
    void fields() {
        for (var e : shared.entrySet()) { if (e.getValue().isEmpty()) { shared.remove(e.getKey()); } }
        for (var e : plain.entrySet()) { if (e.getValue().isEmpty()) { plain.remove(e.getKey()); } } // expect
        // replacing a value keeps the structure
        for (String k : plain.keySet()) { plain.put(k, ""); }
    }
    void reassigned(Collection<String> c, List<String> other) {
        c = new java.util.concurrent.CopyOnWriteArrayList<>(c);
        for (String s : c) { c.remove(s); }
    }
    void leavesLater(List<String> l) {
        for (String s : l) {
            if (s.isEmpty()) {
                l.remove(s);
                System.out.println(s);
                break;
            }
        }
    }
}
