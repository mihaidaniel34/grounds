// grounds: test
import static org.junit.jupiter.api.Assertions.*;
class ATest {
    void t() {
        assertThrows(IllegalStateException.class, () -> { prepare(); run(); }); // expect
        assertThrows(IllegalStateException.class, () -> run());
        assertThrows(IllegalStateException.class, () -> run(build()));
    }
    void prepare() {}
    void run() {}
    void run(Object o) {}
    Object build() { return null; }
}
