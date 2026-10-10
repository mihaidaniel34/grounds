import java.time.format.DateTimeFormatter;
class A {
    void m() {
        DateTimeFormatter a = DateTimeFormatter.ofPattern("yyyy-MM-DD"); // expect
        DateTimeFormatter ok = DateTimeFormatter.ofPattern("yyyy-DDD");
    }
}
