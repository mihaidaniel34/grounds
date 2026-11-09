import java.util.*;
class A {
    boolean m(List<String> l) {
        boolean a = l.stream().filter(String::isEmpty).count() > 0; // expect
        // exactly one, and a bounded count, are not existence tests
        boolean one = l.stream().filter(String::isEmpty).count() == 1;
        boolean two = l.stream().limit(2).count() > 0;
        boolean some = l.stream().filter(String::isEmpty).count() >= 1; // expect
        return l.stream().count() == 5;
    }
}
