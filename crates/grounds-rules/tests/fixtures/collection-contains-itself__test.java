// grounds: test
import java.util.*;
class CycleTest {
    void printsCycle(List<Object> a) {
        // the cycle is the input under test
        a.add(a);
        assert a.toString() != null;
        assert a.containsAll(a); // expect
    }
}
