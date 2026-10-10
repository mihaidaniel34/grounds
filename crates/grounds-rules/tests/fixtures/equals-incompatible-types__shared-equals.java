// grounds: no-findings
import java.util.*;
// List defines equality across implementations
class A {
    boolean m() {
        return new ArrayList<String>().equals(new LinkedList<String>());
    }
}
