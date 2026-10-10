import java.util.*;
class A {
    void m(List<String> l) {
        String[] a = (String[]) l.toArray(); // expect
        String[] b = l.toArray(new String[0]);
        Object[] c = (Object[]) l.toArray();
    }
}
