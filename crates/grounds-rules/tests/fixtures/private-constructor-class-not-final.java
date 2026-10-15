import lombok.Value;
import lombok.experimental.NonFinal;
class Singleton { // expect
    private static final Singleton I = new Singleton();
    private Singleton() {}
}
final class Done { private Done() {} }
class Outer {
    private Outer() {}
    static class Inner extends Outer {}
}
class Holder {
    private static class Implicit { int x; } // expect
    private static class Anon {}
    Object a = new Anon() {};
}
class Pool extends java.util.concurrent.ThreadPoolExecutor { // expect
    private Pool() { super(1, 1, 0L, java.util.concurrent.TimeUnit.SECONDS, new java.util.concurrent.LinkedBlockingQueue<>()); }
}
class Base<T> { private Base() {} static class Sub extends Base<String> {} }
class Values {
    // @Value makes the class final; @NonFinal keeps it extensible
    @Value private static class Pair { String left; String right; }
    @Value @NonFinal private static class Open { String key; } // expect
}
