import java.util.List;

class Describer {
    // a `T` is no `Class<?>`: javac calls describe(Object) for `t`
    private static String describe(Class<?> c) { return "class"; }
    private static String describe(Object o) { return "object"; }
    private static String describeOnce(Class<?> c) { return "class"; } // expect
    private static String describeOnce(Object o) { return "object"; }
    <T> String both(T t) { return describe(t) + describe(String.class) + describeOnce(t); }

    // a `T extends Number` is a Number, not an int: javac calls number(Number)
    private static void number(Number n) { }
    private static void number(Object o) { } // expect
    private static void number(int i) { } // expect
    <T extends Number> void bounded(T t) { number(t); }

    // a `K[]` is no `String[]`, a `K` no `String`: the Object... overload
    private static String text(String... s) { return ""; } // expect
    private static String text(Object... o) { return ""; }
    <K> void arrays(K key, K[] keys) { text(key); text(keys); }

    // a bound that is a box type unboxes
    private static void count(int n) { }
    private static void count(String s) { } // expect
    <N extends Integer> void boxed(N n) { count(n); }

    // an unbounded `E` is a list only where it is declared one
    private static void size(List<?> l) { } // expect
    private static void size(Object o) { }
    <E> void element(E e) { size(e); }
}
