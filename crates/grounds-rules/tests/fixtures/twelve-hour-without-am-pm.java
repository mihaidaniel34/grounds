import java.text.SimpleDateFormat;
class A {
    void m() {
        SimpleDateFormat a = new SimpleDateFormat("hh:mm"); // expect
        SimpleDateFormat ok = new SimpleDateFormat("hh:mm a");
        SimpleDateFormat ok2 = new SimpleDateFormat("HH:mm");
    }
}
