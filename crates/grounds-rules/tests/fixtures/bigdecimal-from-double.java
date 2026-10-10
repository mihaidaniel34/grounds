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
