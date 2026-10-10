import javax.net.ssl.*;
class A implements HostnameVerifier {
    public boolean verify(String host, SSLSession s) { return true; } // expect
}
class B {
    HostnameVerifier v = (h, s) -> true; // expect
}
