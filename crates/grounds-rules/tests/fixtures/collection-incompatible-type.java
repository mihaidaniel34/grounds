import java.util.*;
class A {
    void m(List<String> names, Map<String, Integer> ages, Set<Long> ids, Long id, int n, Map<Long, String> byId, Collection<? extends Number> nums) {
        names.contains(id); // expect
        ages.get(id); // expect
        ages.containsKey(5); // expect
        ids.contains("x"); // expect
        ids.remove(n); // expect
        byId.get(n); // expect
        names.remove(0);
        names.contains("x");
        ids.contains(id);
        byId.get(id);
        ages.containsValue(3);
        nums.contains(3);
        ages.remove("k");
    }
}
