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
class CookieCases {
    void request(Req request) {
        request.setCookies(new javax.servlet.http.Cookie("session", "x"));
    }
    void dynamic(boolean https) {
        javax.servlet.http.Cookie c = new javax.servlet.http.Cookie("s", "v");
        c.setSecure(https);
        c.setHttpOnly(true);
    }
    interface Req { void setCookies(javax.servlet.http.Cookie... c); }
}
