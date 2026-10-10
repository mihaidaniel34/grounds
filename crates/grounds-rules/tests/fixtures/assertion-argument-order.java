// grounds: test
import static org.junit.jupiter.api.Assertions.*;
import org.junit.jupiter.api.Test;
class ATest {
    @Test void t() {
        int result = compute();
        assertEquals(result, 42); // expect
        assertEquals(42, result);
        assertEquals(result, "x".length());
    }
    int compute() { return 42; }
}
class BTest {
    @org.testng.annotations.Test void t() {
        int r = 1;
        org.testng.Assert.assertEquals(r, 1);
        org.testng.Assert.assertEquals(1, r); // expect
    }
}
