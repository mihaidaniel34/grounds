import java.text.*;
import java.util.Calendar;
import java.time.format.DateTimeFormatter;
class A {
    private static final SimpleDateFormat FMT = new SimpleDateFormat("yyyy"); // expect
    private static Calendar CAL = Calendar.getInstance(); // expect
    private static final DateTimeFormatter OK = DateTimeFormatter.ISO_DATE;
    private final SimpleDateFormat instanceOk = new SimpleDateFormat("yyyy");
}
class GuardedFormat {
    private static final java.text.SimpleDateFormat FORMAT = new java.text.SimpleDateFormat("yyyy");
    static synchronized String format(java.util.Date d) { return FORMAT.format(d); }
    static String parse(String s) throws java.text.ParseException {
        synchronized (GuardedFormat.class) { return FORMAT.parse(s).toString(); }
    }
}
class UnguardedFormat {
    private static final java.text.SimpleDateFormat FORMAT = new java.text.SimpleDateFormat("yyyy"); // expect
    static synchronized String format(java.util.Date d) { return FORMAT.format(d); }
    static String plain(java.util.Date d) { return FORMAT.format(d); }
}
class LocalOfTheSameName {
    // a local named like the field does not stop the guarded field's suppression
    private static final java.text.SimpleDateFormat FORMAT = new java.text.SimpleDateFormat("yyyy");
    static synchronized String format(java.util.Date d) { return FORMAT.format(d); }
    static String other() { String FORMAT = "x"; return FORMAT; }
}
class EscapingLambda {
    private static final java.text.SimpleDateFormat FORMAT = new java.text.SimpleDateFormat("yyyy"); // expect
    // the lambda runs after the monitor is released
    static synchronized Runnable task(java.util.Date d) { return () -> FORMAT.format(d); }
}
