class Point {
    Point(int x, int y) {}
}
@lombok.RequiredArgsConstructor
class Holder { private final String a; private final int b; }
class Use {
    void m() {
        new Point(1); // expect
        new Point(1, 2);
        new Holder("x", 1);
        new Holder("x"); // expect
        new StringBuilder(1, 2, 3);
    }
}
