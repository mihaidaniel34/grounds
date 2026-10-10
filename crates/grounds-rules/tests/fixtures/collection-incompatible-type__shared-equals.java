// grounds: no-findings
import java.util.*;
class A {
    boolean m() {
        List<ArrayList<String>> lists = List.of(new ArrayList<>());
        return lists.contains(new LinkedList<String>());
    }
}
