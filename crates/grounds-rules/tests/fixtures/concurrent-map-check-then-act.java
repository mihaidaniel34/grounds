import java.util.*;
import java.util.concurrent.*;
class A {
    final ConcurrentMap<String, Integer> m = new ConcurrentHashMap<>();
    final Map<String, Integer> plain = new HashMap<>();
    void bad(String k) {
        if (!m.containsKey(k)) { // expect
            m.put(k, 1);
        }
    }
    void ok(String k) {
        if (!plain.containsKey(k)) plain.put(k, 1);
        m.putIfAbsent(k, 1);
    }
}
