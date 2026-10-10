import java.util.TimeZone;
import java.time.ZoneId;
class A {
    void m() {
        TimeZone a = TimeZone.getTimeZone("America/NewYork");
        TimeZone b = TimeZone.getTimeZone("Eastern Time"); // expect
        ZoneId c = ZoneId.of("Mars/Olympus"); // expect
        ZoneId ok = ZoneId.of("Europe/Bucharest");
        TimeZone ok2 = TimeZone.getTimeZone("UTC");
        TimeZone ok3 = TimeZone.getTimeZone("GMT+02:00");
    }
}
