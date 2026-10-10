import java.util.*;
class A {
    void m(List<String> l, HashMap<String, String> m) {
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
}
