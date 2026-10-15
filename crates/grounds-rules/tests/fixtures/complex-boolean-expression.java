import java.util.List;
import java.util.function.Predicate;

class A {
    boolean m(boolean a, boolean b, boolean c, boolean d, boolean e) {
        if (a && b || c && d || e) return true; // expect
        if (a && b || c && d) return true;
        return a && b;
    }

    void set(boolean v) { }

    void args(boolean a, boolean b, boolean c, boolean d, boolean e) {
        set(a || b || c || d || e); // expect
        set(a || b || c || d);
        set(a && check(b && c && d && e));
    }

    boolean check(boolean v) { return v; }

    Predicate<Integer> lambdas(List<Integer> xs) {
        Predicate<Integer> p = x -> x > 1 && x < 9 && x != 3 && x != 5 && x != 7; // expect
        Predicate<Integer> q = x -> x > 1 && x < 9 && x != 3 && x != 5;
        xs.removeIf(x -> {
            return x == 1 || x == 2 || x == 3 || x == 4 || x == 5; // expect
        });
        return p.and(q);
    }
}
