import java.text.SimpleDateFormat;
import java.time.format.DateTimeFormatter;
class A {
    void m() {
        SimpleDateFormat a = new SimpleDateFormat("YYYY-MM-dd"); // expect
        DateTimeFormatter b = DateTimeFormatter.ofPattern("dd/MM/YYYY"); // expect
        SimpleDateFormat ok = new SimpleDateFormat("yyyy-MM-dd");
        DateTimeFormatter week = DateTimeFormatter.ofPattern("YYYY-'W'ww");
    }
}
