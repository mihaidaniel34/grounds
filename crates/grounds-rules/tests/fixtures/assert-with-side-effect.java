import java.util.*;
class A {
    void m(List<String> l, int x) {
        assert l.remove("x"); // expect
        assert x++ > 0; // expect
        assert !l.isEmpty();
    }
}
