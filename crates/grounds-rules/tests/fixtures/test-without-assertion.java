// grounds: test
import static org.junit.jupiter.api.Assertions.*;
import org.junit.jupiter.api.Test;
class ATest {
    @Test void noAssert() { compute(); } // expect
    @Test void withAssert() { assertEquals(1, compute()); }
    @Test void helper() { checkResult(compute()); }
    @org.junit.Test(expected = IllegalStateException.class) void expected() { compute(); }
    @Test void fails() { try { compute(); fail("x"); } catch (Exception e) { } }
    void checkResult(int r) { assertTrue(r > 0); }
    int compute() { return 1; }
}
