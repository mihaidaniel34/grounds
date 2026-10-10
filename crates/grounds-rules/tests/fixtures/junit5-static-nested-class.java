// grounds: test
import org.junit.jupiter.api.*;
class OuterTest {
    @Nested static class Skipped { @Test void t() { } } // expect
    @Nested class Runs { @Test void t() { } }
}
