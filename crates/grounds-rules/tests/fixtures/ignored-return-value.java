import java.math.BigDecimal;
import java.time.LocalDate;
import java.time.format.DateTimeParseException;
import java.util.*;
class A {
    void m(String s, BigDecimal b, LocalDate d, Optional<String> o, List<String> l, StringBuilder sb) {
        s.trim(); // expect
        b.add(BigDecimal.ONE); // expect
        d.plusDays(1); // expect
        l.contains("x"); // expect
        s = s.trim();
        sb.append("x");
        l.add("y");
        o.ifPresent(System.out::println);
        int[] calls = {0};
        o.map(x -> ++calls[0]); // the callback runs
        UUID.fromString(s); // validates by throwing
    }
    boolean valid(String date) {
        try {
            LocalDate.parse(date);
            return true;
        } catch (DateTimeParseException e) {
            return false;
        }
    }
}
