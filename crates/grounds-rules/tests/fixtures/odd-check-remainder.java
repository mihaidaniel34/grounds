import java.util.List;
class A {
    boolean m(int x, long y, List<String> l) {
        if (x % 2 == 1) return true; // expect
        if (1 != y % 2) return true; // expect
        if (l.size() % 2 == 1) return true;
        return x % 2 != 0;
    }
}
