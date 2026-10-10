// grounds: test
import static org.junit.jupiter.api.Assertions.*;
import org.junit.jupiter.api.Test;
class ATest {
    @Test void noAssert() { compute(); } // expect
    @Test void withAssert() { assertEquals(1, compute()); }
    @Test void helper() { checkResult(compute()); }
    @org.junit.Test(expected = IllegalStateException.class) void expected() { compute(); }
    @Test void fails() { try { compute(); fail("x"); } catch (Exception e) { } }
    @Test void mockMvc(Mvc mvc) { mvc.perform("/").andExpect(200); }
    @Test void onlyPrints(Mvc mvc) { mvc.perform("/").andDo(); } // expect
    @Test void contextLoads() { }
    interface Mvc { Mvc perform(String u); Mvc andExpect(int s); Mvc andDo(); }
    void checkResult(int r) { assertTrue(r > 0); }
    int compute() { return 1; }
}
