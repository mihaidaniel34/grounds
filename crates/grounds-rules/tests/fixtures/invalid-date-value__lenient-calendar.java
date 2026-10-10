import java.util.*;
class A {
    void m() {
        Calendar c = new GregorianCalendar(2024, Calendar.MARCH, 1);
        c.set(Calendar.DAY_OF_MONTH, 0); // last day of February
        c.set(Calendar.MONTH, 12); // expect
    }
}
