import java.util.*;
class A {
    void m(List<String> l) {
        l.stream().forEach(System.out::println); // expect
        l.forEach(System.out::println);
        l.stream().filter(String::isEmpty).forEach(System.out::println);
    }
}
