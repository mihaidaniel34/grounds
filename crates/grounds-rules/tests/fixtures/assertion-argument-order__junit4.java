// grounds: test
import static org.junit.Assert.assertEquals;
import org.junit.Test;
class ATest {
    @Test void t(CodedException e) {
        int code = e.getCode();
        // JUnit 4 takes the message first: 0 is the expected value
        assertEquals("unexpected code " + e.getCode(), 0, e.getCode());
        assertEquals("unexpected code " + code, 0, code);
        assertEquals("unexpected code", code, 0); // expect
        assertEquals(code, 0); // expect
    }
}
