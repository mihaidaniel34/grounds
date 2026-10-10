import java.util.*;
class A {
    String m(Optional<String> o) {
        String a = o.orElse(compute()); // expect
        String b = o.orElse(new String("x")); // expect
        String c = o.orElse("x");
        String d = o.orElseGet(this::compute);
        return o.orElse(null);
    }
    String compute() { return ""; }
}
