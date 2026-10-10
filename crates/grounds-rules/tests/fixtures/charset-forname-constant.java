import java.nio.charset.Charset;
class A {
    Charset m() {
        Charset a = Charset.forName("UTF-8"); // expect
        Charset b = Charset.forName("windows-1252");
        return Charset.forName("ISO-8859-1"); // expect
    }
}
