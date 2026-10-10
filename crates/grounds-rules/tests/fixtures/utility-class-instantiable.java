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
