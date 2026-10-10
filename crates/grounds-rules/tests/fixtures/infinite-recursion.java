class A {
    private String name;
    String getName() { return getName(); } // expect
    int fact(int n) { return n <= 1 ? 1 : n * fact(n - 1); }
    void loop(int x) { // expect
        System.out.println(x);
        loop(x + 1);
    }
    void delegate(A other) { other.delegate(this); }
    void guarded(int x) { if (x > 0) guarded(x - 1); }
}
// the lambda overload is inherited from a type we cannot see
class Filter extends missing.BaseFilter {
    Filter version(int value) { return version(f -> f.eq(value)); }
}
class ListUtil {
    @SafeVarargs static <A> java.util.List<A> concat(java.util.List<A>... lists) { return null; }
    static <A> java.util.List<A> concat(A item, java.util.List<A> list) { return concat(java.util.List.of(item), list); }
}
class Spin {
    void lambdas(Runnable... work) { lambdas(() -> {}); } // expect
    <T> void spin(T value) { spin(value); } // expect
    void spin() {}
}
class BaseConcat {
    @SafeVarargs static <T> java.util.List<T> concat(java.util.List<T>... lists) { return null; }
}
class ChildConcat extends BaseConcat {
    static <T> java.util.List<T> concat(T item, java.util.List<T> list) { return concat(java.util.List.of(item), list); }
}
