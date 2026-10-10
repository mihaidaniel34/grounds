import java.util.*;
class A {
    void m(int[] ints, Integer[] boxed) {
        List<int[]> a = Arrays.asList(ints); // expect
        List<Integer> b = Arrays.asList(boxed);
        List<int[]> c = List.of(ints); // expect
    }
}
