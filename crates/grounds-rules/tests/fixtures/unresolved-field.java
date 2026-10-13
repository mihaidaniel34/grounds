class Config {
    static final int MAX = 1;
    int size;
    static class Nested {}
}
class Use {
    void m(Config c, int[] arr) {
        int a = Config.MAXIMUM; // expect
        int b = c.sizes; // expect
        int d = c.size + Config.MAX + arr.length;
        Object n = new Config.Nested();
    }
}
// qualified super reaches a private field of the same top-level class
class QTest {
    private int x;
    class Sub extends QTest {
        class Inner {
            int f() { return Sub.super.x; }
        }
    }
}
