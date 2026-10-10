interface Api { void run(String s); default void other() {} }
abstract class Base { abstract int size(); }
class Missing extends Base implements Api { } // expect
class Complete extends Base implements Api {
    public void run(String s) {}
    int size() { return 0; }
}
abstract class StillAbstract implements Api { }
enum Op { PLUS { int apply() { return 1; } }; abstract int apply(); }
class Cmp implements Comparable<Cmp> { public int compareTo(Cmp o) { return 0; } }
