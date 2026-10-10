import java.text.*;
import java.util.Calendar;
import java.time.format.DateTimeFormatter;
class A {
    private static final SimpleDateFormat FMT = new SimpleDateFormat("yyyy"); // expect
    private static Calendar CAL = Calendar.getInstance(); // expect
    private static final DateTimeFormatter OK = DateTimeFormatter.ISO_DATE;
    private final SimpleDateFormat instanceOk = new SimpleDateFormat("yyyy");
}
