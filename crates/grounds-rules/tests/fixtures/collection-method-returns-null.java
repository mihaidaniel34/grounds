import java.util.*;
class A {
    List<String> m(int x) {
        if (x > 0) return new ArrayList<>();
        return null; // expect
    }
    String[] arr() { return null; } // expect
    Map<String, String> map() { return null; } // expect
    @javax.annotation.Nullable List<String> ok() { return null; }
    String s() { return null; }
}
