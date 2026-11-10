// grounds: test, grounds: no-findings
import java.math.BigDecimal;
class ParseTest {
    void t(double actual) {
        // the exact binary value is what the test compares
        double shifted = new BigDecimal(actual).movePointLeft(301).doubleValue();
        assert shifted > 0;
    }
}
