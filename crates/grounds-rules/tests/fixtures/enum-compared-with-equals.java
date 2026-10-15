enum Color {
    RED, GREEN;
    boolean isRed() { return this.equals(RED); }
    boolean same(Color c) { return equals(c); }
    boolean green(Color c) { return GREEN.equals(c); }
}
class A {
    boolean m(Color a, Color b, Object o) {
        if (a.equals(b)) return true; // expect
        if (a.equals(Color.RED)) return true; // expect
        if (Color.RED.equals(a)) return true;
        if ((Color.GREEN).equals(b)) return true;
        return a == b || a.equals(o);
    }
}
