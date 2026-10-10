// grounds: test
import org.junit.jupiter.api.Test;
class ATest {
    @Test void t() throws Exception {
        Thread.sleep(100); // expect
    }
    void helper() throws Exception { Thread.sleep(1); }
}
