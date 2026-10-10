import java.time.*;
import java.util.Calendar;
class A {
    void m(Calendar cal) {
        LocalDate a = LocalDate.of(2024, 13, 1); // expect
        LocalDate b = LocalDate.of(2024, 2, 32); // expect
        LocalTime c = LocalTime.of(24, 0); // expect
        cal.set(Calendar.MONTH, 12); // expect
        Month d = Month.of(0); // expect
        LocalDate ok = LocalDate.of(2024, 12, 31);
        LocalDate ok2 = LocalDate.of(2024, Month.MARCH, 1);
        cal.set(Calendar.MONTH, 11);
    }
}
