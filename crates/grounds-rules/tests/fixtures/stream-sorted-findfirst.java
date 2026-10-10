import java.util.*;
class A {
    Optional<String> m(List<String> l) {
        Optional<String> a = l.stream().sorted().findFirst(); // expect
        return l.stream().min(Comparator.naturalOrder());
    }
}
