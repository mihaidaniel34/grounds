import java.util.*;
class A {
    void m(List<String> l) {
        for (String s : l) {
            System.out.println(s);
            continue; // expect
        }
        if (l.isEmpty()) return;
        System.out.println(l);
        return; // expect
    }
}
