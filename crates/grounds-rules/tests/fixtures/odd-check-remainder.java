import java.util.List;
class A {
    boolean m(int x, long y, List<String> l) {
        if (x % 2 == 1) return true; // expect
        if (1 != y % 2) return true; // expect
        if (l.size() % 2 == 1) return true;
        // counts up from zero: never negative
        for (int i = 0; i < l.size(); i++) { if (i % 2 == 1) return true; }
        int seen = 0;
        seen += 2;
        if (seen % 2 == 1) return true;
        int down = 0;
        down--;
        if (down % 2 == 1) return true; // expect
        return x % 2 != 0;
    }
}
