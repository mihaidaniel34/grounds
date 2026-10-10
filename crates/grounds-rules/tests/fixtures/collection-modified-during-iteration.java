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
    }
    // declared by interface: may be a CopyOnWriteArrayList
    void param(List<String> l) {
        for (String s : l) { l.remove(s); }
        List<String> cow = new java.util.concurrent.CopyOnWriteArrayList<>(List.of("x"));
        for (String s : cow) { cow.remove(s); }
    }
}
