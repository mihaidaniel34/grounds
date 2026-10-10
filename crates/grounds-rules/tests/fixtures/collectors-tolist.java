import java.util.*;
import java.util.stream.*;
class A {
    List<String> m(List<String> l) {
        return l.stream().map(String::trim).collect(Collectors.toList()); // expect
    }
}
