// grounds: no-findings
import java.util.stream.*;
class A {
    void m() {
        int[] closed = {0};
        try (Stream<String> s = Stream.of("x")) {
            s.onClose(() -> closed[0]++);
            s.forEach(System.out::println);
        }
    }
}
