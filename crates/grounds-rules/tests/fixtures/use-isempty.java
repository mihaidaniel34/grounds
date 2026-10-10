import java.util.*;
class A {
    boolean m(List<String> l, Map<String, String> m, String s, int[] a) {
        if (l.size() == 0) return true; // expect
        if (m.size() > 0) return true; // expect
        if (0 != l.size()) return true; // expect
        if (s.length() == 0) return true; // expect
        if (l.size() >= 1) return true; // expect
        if (l.size() == 1) return true;
        if (a.length == 0) return true;
        return l.isEmpty();
    }
}
