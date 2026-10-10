import java.util.*;
class A {
    boolean m(String a, String b, Comparator<String> c, Integer x, Integer y) {
        if (a.compareTo(b) == 1) return true; // expect
        if (-1 == c.compare(a, b)) return true; // expect
        if (x.compareTo(y) != 1) return true; // expect
        return a.compareTo(b) == 0 || a.compareTo(b) > 0;
    }
}
