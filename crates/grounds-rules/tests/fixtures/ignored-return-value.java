import java.math.BigDecimal;
import java.time.LocalDate;
import java.util.*;
class A {
    void m(String s, BigDecimal b, LocalDate d, Optional<String> o, List<String> l, StringBuilder sb) {
        s.trim(); // expect
        b.add(BigDecimal.ONE); // expect
        d.plusDays(1); // expect
        o.map(String::length); // expect
        l.contains("x"); // expect
        s = s.trim();
        sb.append("x");
        l.add("y");
        o.ifPresent(System.out::println);
    }
}
