// grounds: test
import static org.junit.jupiter.api.Assertions.*;
class ATest {
    void t(String a, String b, Object o, int x) {
        assertTrue(a.equals(b)); // expect
        assertTrue(o == null); // expect
        assertFalse(a == b); // expect
        assertEquals(null, o); // expect
        assertTrue(x == 3); // expect
        assertTrue(x > 3);
        assertEquals(a, b);
    }
}
