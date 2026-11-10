import java.math.BigDecimal;
class A {
    void m(double d, int i) {
        BigDecimal a = new BigDecimal(0.1); // expect
        BigDecimal b = new BigDecimal(d); // expect
        BigDecimal ok = BigDecimal.valueOf(d);
        BigDecimal ok2 = new BigDecimal("0.1");
        BigDecimal ok3 = new BigDecimal(i);
    }
}
class Exact {
    void m(double d, int count, java.math.MathContext mc) {
        // the double is exactly the written value: nothing is rounded
        BigDecimal zero = new BigDecimal(0.00);
        BigDecimal half = new BigDecimal(0.5);
        BigDecimal big = new BigDecimal(2.5e3);
        BigDecimal max = new BigDecimal(Double.MAX_VALUE);
        BigDecimal min = new BigDecimal(-Float.MAX_VALUE);
        BigDecimal whole = new BigDecimal((double) count);
        // rounded on purpose to an explicit precision
        BigDecimal rounded = new BigDecimal(d, mc);
        // still reported: not representable in binary
        BigDecimal tenth = new BigDecimal(0.1f); // expect
        BigDecimal price = new BigDecimal(19.99); // expect
        BigDecimal third = new BigDecimal(1.0 / 3); // expect
        BigDecimal unrounded = new BigDecimal(0.1, java.math.MathContext.UNLIMITED); // expect
        BigDecimal big = new BigDecimal((double) Long.MAX_VALUE); // expect
    }
}
