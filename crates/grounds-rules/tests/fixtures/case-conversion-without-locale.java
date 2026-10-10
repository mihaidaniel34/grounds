import java.util.Locale;
class A {
    String m(String s) {
        String a = s.toLowerCase(); // expect
        return s.toUpperCase(Locale.ROOT);
    }
}
