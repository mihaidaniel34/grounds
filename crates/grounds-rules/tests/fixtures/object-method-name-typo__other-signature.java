import java.util.Set;
class Index<K> {
    public Set<String> equal(K key) { return Set.of(); } // a query, not a typo
    public boolean equal(Object other) { return false; } // expect
}
