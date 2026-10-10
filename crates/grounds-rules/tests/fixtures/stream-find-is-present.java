import java.util.*;
class A {
    boolean m(List<String> l) {
        boolean a = l.stream().filter(String::isEmpty).findFirst().isPresent(); // expect
        boolean b = l.stream().anyMatch(String::isEmpty);
        return l.stream().filter(s -> s.length() > 2).findAny().isEmpty(); // expect
    }
}
