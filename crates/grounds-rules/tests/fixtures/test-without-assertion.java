// grounds: test
import static org.junit.jupiter.api.Assertions.*;
import org.junit.jupiter.api.Nested;
import org.junit.jupiter.api.Test;
import static org.springframework.test.web.servlet.result.MockMvcResultMatchers.status;
import p.Harness;
import p.Inventory;
import testkit.StatusHarness;
import testkit.WebHarness;
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
abstract class ExpressionHarness { void evaluate(String expr, Object expected) { assertEquals(expected, expr); } }
class EvaluationTest extends ExpressionHarness {
    @Nested class Operators {
        @Test void elvis() { evaluate("a ?: b", "a"); }
        @Test void unchecked() { "a".trim(); } // expect
    }
}
class ApiTest extends WebHarness {
    @Test void created() { post("/items", "{}", String.class, 201); }
    @Test void fetchOnly() { fetch("/items"); } // expect
    @Test void resetOnly() { reset(); } // expect
}
class ExchangeTest implements Harness {
    @Test void timeout() { exchange("/items", String.class, status().is(504)); }
    @Nested class Rename {
        @Test void renames() { rewriteRun("a", "b"); }
    }
}
class InventoryTest {
    @Test void adds() { new Inventory().add("x"); } // expect
}
class HelperChainTest {
    @Test void deep() { level1(); }
    void level1() { level2(); }
    void level2() { level3(); }
    void level3() { assertTrue(true); }
    @Test void cyclic() { ping(1); } // expect
    void ping(int n) { pong(n); }
    void pong(int n) { ping(n); }
}
class StatusTest extends StatusHarness {
    @Test void created() { expectCreated("/items"); }
    @Test void posted() { post("/items", "{}", String.class); }
}
class CallbackTest {
    @Test void verified() { decode("[]", Verifier::verifyComplete); }
    @Test void mapped() { decode("[]", Verifier::complete); } // expect
    @Test void awaited() { awaitCount(3); }
    void decode(String input, java.util.function.Consumer<Verifier> step) {}
    void awaitCount(int n) { await().atMost(5).until(() -> n, c -> c == 3); }
    Awaiter await() { return new Awaiter(); }
    interface Verifier { void verifyComplete(); void complete(); }
    static class Awaiter { Awaiter atMost(int s) { return this; } <T> T until(java.util.function.Supplier<T> s, java.util.function.Predicate<T> p) { return null; } }
}
