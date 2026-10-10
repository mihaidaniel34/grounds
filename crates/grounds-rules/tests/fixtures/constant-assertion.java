// grounds: test
import static org.junit.jupiter.api.Assertions.*;
class ATest {
    void t(Object o) {
        assertTrue(true); // expect
        assertNotNull(new Object()); // expect
        assertTrue(false); // expect
        assertNotNull(o);
    }
}
