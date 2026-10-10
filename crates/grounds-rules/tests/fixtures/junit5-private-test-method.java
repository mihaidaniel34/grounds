// grounds: test
import org.junit.jupiter.api.*;
class ATest {
    @Test private void hidden() { } // expect
    @Test static void st() { } // expect
    @Test void ok() { }
    @BeforeEach private void setup() { } // expect
}
