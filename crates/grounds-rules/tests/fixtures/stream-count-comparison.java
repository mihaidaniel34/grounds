import java.util.*;
class A {
    boolean m(List<String> l) {
        boolean a = l.stream().filter(String::isEmpty).count() > 0; // expect
        return l.stream().count() == 5;
    }
}
