enum Color { RED, GREEN }
class A {
    boolean m(Color a, Color b, Object o) {
        if (a.equals(b)) return true; // expect
        if (a.equals(Color.RED)) return true; // expect
        return a == b || a.equals(o);
    }
}
