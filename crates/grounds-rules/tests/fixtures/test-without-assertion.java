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
abstract class HarnessBase { void rewriteRun(String before, String after) { if (!before.equals(after)) throw new AssertionError(); } }
class RecipeTest extends HarnessBase {
    @Test void inherited() { rewriteRun("a", "a"); }
    @Test void unresolvedBase() { missingHelper(); }
    @Test void staticHelperOnly() { java.util.Collections.emptyList(); } // expect
}
class Computing { int compute() { return 42; } }
class ComputeTest extends Computing {
    @Test void bad() { compute(); } // expect
    @Test void own() { helper(); } // expect
    void helper() { String assertionMessage = "unused"; }
}
class OverloadTest {
    @Test void test() { exercise(); } // expect
    void exercise() { inspect(1); }
    void inspect(int x) { }
    void inspect() { assertTrue(true); }
}
class FirstHopTest {
    @Test void test() { exercise(1); } // expect
    void exercise(int x) { System.out.println(x); }
    void exercise() { inspect(); }
    void inspect() { assert true; }
}
class VarargsHelperTest {
    @Test void none() { expectFailure("msg"); }
    @Test void many() { expectFailure("msg", "a", "b"); }
    private static void expectFailure(String message, String... hosts) { assertEquals(message, String.join(",", hosts)); }
}
