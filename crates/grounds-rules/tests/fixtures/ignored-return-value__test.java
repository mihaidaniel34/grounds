// grounds: test
import static org.junit.jupiter.api.Assertions.*;
import java.util.*;
class ThrowingTest {
    void t(List<String> l, String s) {
        // called for the exception
        try { l.size(); } catch (Exception e) { assertNotNull(e); }
        try {
            l.size();
            fail();
        } catch (Exception e) {
            assertNotNull(e);
        }
        assertThrows(NullPointerException.class, () -> { s.trim(); });
        // not the statement that is expected to throw
        try { s.trim(); l.clear(); } catch (Exception e) { } // expect
        assertThrows(NullPointerException.class, () -> { s.trim(); l.clear(); }); // expect
        s.trim(); // expect
    }
}
