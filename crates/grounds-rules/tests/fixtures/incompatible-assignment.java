import java.util.*;
class Util {
    static Object name() { return "n"; }
    static String text() { return "t"; }
    static Number count() { return 1; }
    static ArrayList<String> list() { return null; }
    static <T> T any() { return null; }
    static int prim() { return 1; }
}
class Use {
    String f = Util.name(); // expect
    void m() {
        String a = Util.name(); // expect
        Object b = Util.text();
        CharSequence c = Util.text();
        Integer d = (Integer) Util.count();
        Integer e = Util.count(); // expect
        List<String> l = Util.list();
        String g = Util.any();
        long p = Util.prim();
        var v = Util.name();
        String s;
        s = Util.name(); // expect
        s = Util.text();
        StringBuilder sb = new StringBuilder();
        Runnable r = Util.text(); // expect
    }
}
class Overloads {
    static Object pick(List<Integer> x) { return null; }
    static String pick(Collection<String> x) { return ""; }
    private static Object hidden(Object o) { return null; }
    static String hidden(String s) { return ""; }
    String s = pick(List.of("x"));
    String t = hidden("x");
}
class FieldTarget {
    static class Base { private String field; }
    Object field;
    static Object result() { return null; }
    class Inner extends Base {
        void f() { field = result(); }
    }
}
// the other bound of a type variable may hold the method javac picks
interface IA { Object pick(java.util.List<Integer> xs); }
interface IB { String pick(java.util.Collection<String> xs); }
class Bounds {
    <T extends IA & IB> void f(T t) {
        String s = t.pick(java.util.List.of("x"));
    }
}
// a superclass's private get() is not inherited: PC's get() is IGet's default
interface IGet { default Integer get() { return 1; } }
class PB { private String get() { return ""; } }
class PC extends PB implements IGet {}
class PMain { Integer f(PC c) { Integer v = c.get(); return v; } }
// nestmates share private access, but a private method is still not inherited
class NMain {
    interface I { default Integer get() { return 1; } }
    static class Base { private String get() { return ""; } }
    static class Child extends Base implements I {}
    Integer f(Child c) { Integer value = c.get(); return value; }
}
