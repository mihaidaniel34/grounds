import java.util.Date;
class A {
    long m(Date d) {
        long a = new Date().getTime(); // expect
        return d.getTime();
    }
}
