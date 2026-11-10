// grounds: test
import org.junit.jupiter.api.Test;
class Parser { static Parser parse(String s) { return new Parser(); } }
class NoThrowTests {
    // the name says the test checks only that nothing is thrown
    @Test void shouldNotFailWhenInputIsEmpty() { Parser.parse(""); }
    @Test void acceptsTrailingSlash() { Parser.parse("a/"); }
    @Test void maximumLength_Ok() { Parser.parse("aaaa"); }
    @Test void doesNotThrowOnUnicode() { Parser.parse("é"); }
    // still reported: the name says nothing about throwing
    @Test void parsesPath() { Parser.parse("a/b"); } // expect
    @Test void testSuccess() { Parser.parse("ok"); } // expect
    @Test void okayToParse() { Parser.parse("x"); } // expect
    @Test void testAllowListContains() { Parser.parse("x"); } // expect
    @Test void shouldAllowEmptyName() { Parser.parse(""); }
    @Test void shouldCreateParserWithConstructorThatAcceptsText() { Parser.parse("x"); } // expect
}
class AsyncTests {
    // vert.x unit tests assert through their context
    @Test void completes(io.vertx.ext.unit.TestContext ctx) {
        io.vertx.ext.unit.Async async = ctx.async();
        Parser.parse("x");
        async.complete();
    }
    @Test void notAContext(Parser ctx) { Parser.parse("y"); } // expect
    @Test void contextUnused(io.vertx.ext.unit.TestContext ctx) { Parser.parse("z"); } // expect
}
class GuavaTesters {
    // Guava's EqualsTester throws AssertionError when a group is wrong
    @Test void equality() {
        new com.google.common.testing.EqualsTester().addEqualityGroup("a", "a").testEquals();
    }
    @Test void ownTester() { new Tester().testEquals(); } // expect
    static class Tester { void testEquals() { } }
}
