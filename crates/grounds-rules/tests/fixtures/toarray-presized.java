import java.util.*;
class A {
    String[] m(List<String> l) {
        String[] a = l.toArray(new String[l.size()]); // expect
        return l.toArray(new String[0]);
    }
}
