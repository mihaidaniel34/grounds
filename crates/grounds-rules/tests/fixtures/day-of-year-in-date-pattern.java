import java.time.format.DateTimeFormatter;
class A {
    void m() {
        DateTimeFormatter a = DateTimeFormatter.ofPattern("yyyy-MM-DD"); // expect
        DateTimeFormatter ok = DateTimeFormatter.ofPattern("yyyy-DDD");
        // day of the month and day of the year, both meant
        DateTimeFormatter both = DateTimeFormatter.ofPattern("yyyy-MM-dd 'day' DDD");
    }
}
