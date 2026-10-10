class Utils { // expect
    static int twice(int x) { return 2 * x; }
}
final class Good {
    private Good() {}
    static int twice(int x) { return 2 * x; }
}
class Mixed {
    static int a() { return 1; }
    int b() { return 2; }
}
class Main {
    public static void main(String[] args) {}
}
class Consts { // expect
    public static final int X = 1;
}
class Empty {}
class Instantiated {
    static final int Y = 2;
}
class UsesIt { Object o = new Instantiated(); }
class Extended { static int z() { return 1; } }
class Sub extends Extended {}
class Q { // expect
    static int f() { return 1; }
}
class UsesOtherQ { Object o = new other.Q(); }
