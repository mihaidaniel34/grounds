import java.math.BigDecimal;
class A {
    boolean m(BigDecimal a, BigDecimal b) {
        if (a.equals(b)) return true; // expect
        return a.compareTo(b) == 0;
    }
}
