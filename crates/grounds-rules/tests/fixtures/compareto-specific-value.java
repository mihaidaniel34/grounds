import java.util.*;
class A {
    boolean m(String a, String b, Comparator<String> c, Integer x, Integer y) {
        if (a.compareTo(b) == 1) return true; // expect
        if (-1 == c.compare(a, b)) return true; // expect
        if (x.compareTo(y) != 1) return true; // expect
        return a.compareTo(b) == 0 || a.compareTo(b) > 0;
    }
}
class Exact {
    // BigDecimal and BigInteger document compareTo as returning exactly -1, 0 or 1
    boolean m(java.math.BigDecimal price, java.math.BigDecimal min, java.math.BigInteger a, java.math.BigInteger b) {
        if (price.compareTo(min) == -1) return true;
        if (a.compareTo(b) == 1) return true;
        if (price.compareTo(min) == 2) return true; // expect
        return java.util.Comparator.<java.math.BigDecimal>naturalOrder().compare(price, min) == 1; // expect
    }
}
