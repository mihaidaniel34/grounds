interface Api { void run(String s); <T> T convert(Class<T> c); }
abstract class Base { abstract int size(); protected void hook(java.util.List<String> l) {} }
class Impl extends Base implements Api {
    @Override public void run(String s) {}
    @Override public <T> T convert(Class<T> c) { return null; }
    @Override int size() { return 0; }
    @Override protected void hook(java.util.List<String> l) {}
    @Override public void runNow(String s) {} // expect
    @Override public void run(int x) {} // expect
    @Override public String toString() { return ""; }
}
interface Node<N extends Node<N>> { N next(); }
abstract class Chain<E extends Node<E>> extends java.util.AbstractCollection<E> {
    @Override public boolean remove(Object o) { return false; }
    @Override public boolean add(E e) { return true; }
    @Override public <T> T[] toArray(T[] a) { return a; }
    @Override public boolean contains(E e) { return false; } // expect
}
class RawVersion implements Comparable {
    @Override public int compareTo(Object o) { return 0; }
}
class Length implements java.util.function.Function<String, Integer> {
    @Override public Integer apply(String s) { return s.length(); }
}
class Enclosing<T> {
    class Enclosed<S> { void take(S s, T t) {} }
}
class ExtendsEnclosing extends Enclosing<String> {
    class ExtendsEnclosed extends Enclosed<Integer> {
        @Override void take(Integer s, String t) {}
    }
}
