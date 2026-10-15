class Base {
    void run() {}
    String name() { return ""; }
}
class Sub extends Base {
    void run() {} // expect
    @Override String name() { return "x"; }
    public String toString() { return ""; } // expect
    void own() {}
}
// overloads with a type variable parameter do not override Object parameters
interface Node<N extends Node<N>> { N next(); }
abstract class Chain<E extends Node<E>> extends java.util.AbstractCollection<E> implements java.util.Deque<E> {
    public boolean remove(E e) { return false; }
    public boolean contains(Object o) { return false; } // expect
}
// generic overrides: substituted type arguments, raw supertypes, generic methods
class Version implements Comparable<Version> {
    public int compareTo(Version o) { return 0; } // expect
}
class RawVersion implements Comparable {
    public int compareTo(Object o) { return 0; } // expect
}
class Length implements java.util.function.Function<String, Integer> {
    public Integer apply(String s) { return s.length(); } // expect
}
abstract class Bag<E> extends java.util.AbstractCollection<E> {
    public <T> T[] toArray(T[] a) { return a; } // expect
    public boolean add(E e) { return true; } // expect
}
