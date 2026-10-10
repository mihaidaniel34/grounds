import java.util.*;
class A {
    void m(Map<String, List<String>> m, String k) {
        if (!m.containsKey(k)) { // expect
            m.put(k, new ArrayList<>());
        }
        m.computeIfAbsent(k, x -> new ArrayList<>());
        if (!m.containsKey(k)) { m.put(k, new ArrayList<>()); log(); }
    }
    void log() {}
}
