import java.time.*;
import java.time.temporal.ChronoUnit;
class A {
    void m(Instant i) {
        Instant a = i.plus(1, ChronoUnit.MONTHS); // expect
        Instant b = i.minus(2, ChronoUnit.YEARS); // expect
        Instant ok = i.plus(1, ChronoUnit.DAYS);
    }
}
