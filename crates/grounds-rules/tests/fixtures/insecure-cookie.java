import javax.servlet.http.Cookie;
class A {
    Cookie bad() {
        Cookie c = new Cookie("session", "x"); // expect
        return c;
    }
    Cookie good() {
        Cookie c = new Cookie("session", "x");
        c.setSecure(true);
        c.setHttpOnly(true);
        return c;
    }
}
