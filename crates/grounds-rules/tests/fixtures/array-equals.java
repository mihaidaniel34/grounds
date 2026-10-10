import java.util.*;
class A {
    boolean m(int[] a, int[] b, String s) {
        boolean r = a.equals(b); // expect
        r = Objects.equals(a, b); // expect
        r = Arrays.equals(a, b);
        return r || s.equals("x");
    }
}
