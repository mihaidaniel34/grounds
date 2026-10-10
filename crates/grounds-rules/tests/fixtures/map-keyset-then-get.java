import java.util.*;
class A {
    void m(Map<String, Integer> m) {
        for (String k : m.keySet()) { // expect
            System.out.println(k + m.get(k));
        }
        for (Map.Entry<String, Integer> e : m.entrySet()) { }
        for (String k : m.keySet()) { System.out.println(k); }
    }
}
