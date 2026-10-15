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
class CTest {
    @org.junit.Test void t(CodedException e) {
        // JUnit 4 takes the message first: 0 is the expected value
        org.junit.Assert.assertEquals("unexpected code " + e.getCode(), 0, e.getCode());
        org.junit.Assert.assertEquals("unexpected code", e.getCode(), 0); // expect
        // JUnit 5 takes it last
        org.junit.jupiter.api.Assertions.assertEquals(0, e.getCode(), "unexpected code");
        org.junit.jupiter.api.Assertions.assertEquals(e.getCode(), 0, "unexpected code"); // expect
    }
}
