import java.util.ArrayList;
import java.util.List;

// a `K` is no `int`: javac calls remove(Object), as ListOrderedMap does
class OrderedMap<K> {
    private final List<K> keys = new ArrayList<>();
    K get(int index) { return keys.get(index); }
    void remove(int index) { remove(get(index)); }
    void remove(Object key) { keys.remove(key); }
    // a `K[]` is no `String[]`
    void names(K[] all) { names(all); } // expect
    void names(String[] all) { }
    // the variable itself
    void put(K key) { put(key); } // expect
    void put(String key) { }
}

// an `N` fits `N` itself before it unboxes
class Counter<N extends Integer> {
    void at(N n) { at(n); } // expect
    void at(int n) { }
}
