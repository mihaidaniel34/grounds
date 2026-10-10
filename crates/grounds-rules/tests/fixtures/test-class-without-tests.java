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
