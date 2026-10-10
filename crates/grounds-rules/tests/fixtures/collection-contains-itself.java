import java.util.*;
class A {
    void m(List<Object> l, Set<Object> s) {
        l.add(l); // expect
        s.containsAll(s); // expect
        l.removeAll(l); // expect
        l.add(s);
    }
}
