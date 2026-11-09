// grounds: test
import org.junit.jupiter.api.*;
class EmptyTest { // expect
    void helper() { }
}
class RealTest {
    @Test void t() { }
}
abstract class BaseTest { }
class OuterTest {
    @Nested class Inner { @Test void t() { } }
}
class RoutingJmhTest {
    @org.openjdk.jmh.annotations.Benchmark public void route() { }
}
interface StoreContract { @Test default void stores() { } }
// the tests come from the contract interface
class JdbcStoreContractTest implements StoreContract { }
// shared fixtures for the classes extending it
class FixtureTest { void setUp() { } }
class UsesFixtureTest extends FixtureTest { @Test void t() { } }
