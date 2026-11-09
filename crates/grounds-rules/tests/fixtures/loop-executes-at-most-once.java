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
        // "the first one, if any" is reported as info
        Iterator<String> it = l.iterator();
        while (it.hasNext()) { // expect
            return it.next();
        }
        for (String s : l) {
            if (s.isEmpty()) continue;
            return s;
        }
        return null;
    }
}
