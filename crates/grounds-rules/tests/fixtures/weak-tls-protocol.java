import javax.net.ssl.*;
class A {
    void m(SSLSocket s) throws Exception {
        SSLContext a = SSLContext.getInstance("SSLv3"); // expect
        SSLContext b = SSLContext.getInstance("TLSv1.1"); // expect
        SSLContext ok = SSLContext.getInstance("TLSv1.3");
        SSLContext alias = SSLContext.getInstance("SSL");
        s.setEnabledProtocols(new String[] {"TLSv1"}); // expect
    }
}
