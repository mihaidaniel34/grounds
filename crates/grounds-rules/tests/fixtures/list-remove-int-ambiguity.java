import java.util.*;
class A {
    void m(List<Integer> ids, int id, List<String> names, int i) {
        ids.remove(id); // expect
        ids.remove(i);
        ids.remove(Integer.valueOf(id));
        names.remove(id);
    }
}
