import javax.net.ssl.X509TrustManager;
import java.security.cert.X509Certificate;
class TrustAll implements X509TrustManager {
    public void checkClientTrusted(X509Certificate[] c, String a) { } // expect
    public void checkServerTrusted(X509Certificate[] c, String a) { } // expect
    public X509Certificate[] getAcceptedIssuers() { return new X509Certificate[0]; }
}
