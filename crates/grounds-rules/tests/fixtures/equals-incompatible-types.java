import java.util.*;
class User { Long id; }
interface Named {}
final class Fin {}
class A {
    boolean m(User u, Long id, String s, Integer i, Optional<String> os, Named n, Fin f, Object o, CharSequence cs) {
        boolean r = u.equals(id); // expect
        r = s.equals(i); // expect
        r = os.equals(s); // expect
        r = Objects.equals(s, 42); // expect
        r = f.equals(n); // expect
        r = s.equals(o);
        r = s.equals(cs);
        r = o.equals(u);
        r = n.equals(u);
        r = i.equals(5);
        return r;
    }
    void contract(User u) {
        org.junit.jupiter.api.Assertions.assertFalse(u.equals("not a user"));
    }
}
class Widen {
    boolean m(boolean b, List<String> strings, ArrayList<Integer> ints) {
        Object value = java.util.stream.Stream.of(b).map(flag -> { if (flag) return strings; return ints; }).findFirst().get().get(0);
        return value.equals(1);
    }
}
