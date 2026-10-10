import java.util.*;
class A {
    String m(List<String> l) {
        for (String s : l) { // expect
            System.out.println(s);
            return s;
        }
        while (l.isEmpty()) { // expect
            l.add("x");
            break;
        }
        for (String s : l) {
            if (s.isEmpty()) continue;
            return s;
        }
        return null;
    }
}
