import java.text.SimpleDateFormat;
class A {
    void m() {
        SimpleDateFormat a = new SimpleDateFormat("yyyy-mm-dd"); // expect
        SimpleDateFormat ok = new SimpleDateFormat("HH:mm");
        SimpleDateFormat ok2 = new SimpleDateFormat("yyyy-MM-dd HH:mm");
    }
}
