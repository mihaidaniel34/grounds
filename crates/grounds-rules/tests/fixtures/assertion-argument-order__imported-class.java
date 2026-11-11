// grounds: test
import org.junit.jupiter.api.Assertions;
import org.junit.jupiter.api.Test;
import lib.Checks;
class ImportedTest {
    // JUnit's class without its jar: its import names it
    @Test void t() {
        int r = 1;
        Assertions.assertEquals(r, 42); // expect
        org.junit.jupiter.api.Assertions.assertEquals(r, 43); // expect
        Assertions.assertEquals(42, r);
        // a class of the project or a library that is not JUnit's
        Checks.assertEquals(r, 42);
    }
}
