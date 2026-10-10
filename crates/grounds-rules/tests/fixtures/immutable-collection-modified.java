import java.util.*;
class A {
    void m(List<String> in) {
        List<String> a = List.of("x");
        a.add("y"); // expect
        Map<String, Integer> m = Map.of();
        m.put("k", 1); // expect
        List<String> b = Collections.unmodifiableList(in);
        b.clear(); // expect
        List<String> c = Arrays.asList("a", "b");
        c.add("c"); // expect
        c.set(0, "z");
        List<String> d = new ArrayList<>(List.of("x"));
        d.add("y");
        List<String> e = List.of();
        e = new ArrayList<>();
        e.add("x");
        Collections.emptyList().add(null); // expect
    }
}
