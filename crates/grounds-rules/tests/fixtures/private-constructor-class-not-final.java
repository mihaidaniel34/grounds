class Singleton { // expect
    private static final Singleton I = new Singleton();
    private Singleton() {}
}
final class Done { private Done() {} }
class Outer {
    private Outer() {}
    static class Inner extends Outer {}
}
