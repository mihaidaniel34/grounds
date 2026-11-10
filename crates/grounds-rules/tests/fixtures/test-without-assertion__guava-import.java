// grounds: test
// an on-demand import of Guava's testing package does not make a project's own
// *Tester class Guava's
import org.junit.jupiter.api.Test;
import com.google.common.testing.*;
class OwnTesters {
    static class OrderTester { void testOrder() { } }
    @Test void ownTester() { new OrderTester().testOrder(); } // expect
    @Test void guavaTester() { new NullPointerTester().testAllPublicStaticMethods(OwnTesters.class); }
}
