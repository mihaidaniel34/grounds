// grounds: test; grounds: no-findings
import static org.junit.jupiter.api.Assertions.assertEquals;

import lombok.experimental.Delegate;
import org.junit.jupiter.api.Test;

// @Delegate gives a class the methods of the field's type, which the model
// does not reproduce: here an assertEquals(actual, expected), a member that
// hides the static import (JLS 15.12.1) in the class, its subclasses and
// the classes nested in it. Which assertEquals a call reaches is unknown.
interface Checks {
    void assertEquals(Object actual, Object expected);
}

class Checking {
    @Delegate Checks checks;

    class InnerTest {
        @Test void t(int actual) { assertEquals(actual, 42); }
    }
}

class OrderTest extends Checking {
    @Test void t(int actual) { assertEquals(actual, 42); }
}
