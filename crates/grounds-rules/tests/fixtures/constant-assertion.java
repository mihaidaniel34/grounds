// grounds: test
import static org.junit.jupiter.api.Assertions.*;
class ATest {
    void t(Object o) {
        assertTrue(true); // expect
        assertNotNull("message"); // expect
        // runs the constructor of a utility class for coverage
        assertNotNull(new Object());
        assertTrue(false); // expect
        assertNotNull(o);
    }
}
class MessageFirst {
    void t(int failures, Object list) {
        org.junit.Assert.assertEquals("Value B should not be lost", 0, failures);
        org.junit.Assert.assertNotNull("list must not be null", list);
    }
}
