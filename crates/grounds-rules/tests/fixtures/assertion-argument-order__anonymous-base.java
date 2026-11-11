// grounds: test
import static org.junit.jupiter.api.Assertions.assertEquals;

// an anonymous class whose superclass we cannot see may declare assertEquals,
// which hides the static import
class AnonymousBase {
    void run() {
        new ext.Base() {
            void test(int actual) { assertEquals(actual, 42); }
        };
    }
    void plain(int actual) {
        assertEquals(actual, 42); // expect
    }
}
