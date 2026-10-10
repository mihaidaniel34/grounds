import java.util.*;
class A {
    void m(List<String> l) {
        l.stream().map(String::trim); // expect
        l.stream().filter(s -> s.isEmpty()).peek(System.out::println); // expect
        l.stream().forEach(System.out::println);
        long c = l.stream().count();
    }
}
