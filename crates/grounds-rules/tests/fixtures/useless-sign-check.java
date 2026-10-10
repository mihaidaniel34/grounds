import java.util.*;
class A {
    boolean m(List<String> l, String s, int[] a, Map<String, String> m, int x) {
        if (l.size() < 0) return true; // expect
        if (s.length() >= 0) return true; // expect
        if (0 > a.length) return true; // expect
        if (m.size() >= 0) return true; // expect
        if (l.size() > 0) return true;
        return x < 0;
    }
}
