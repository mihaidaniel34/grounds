// grounds: test grounds: no-findings
import javax.net.ssl.X509TrustManager;
import java.security.cert.X509Certificate;
class TrustAllTestServer implements X509TrustManager {
    public void checkClientTrusted(X509Certificate[] c, String a) { }
    public void checkServerTrusted(X509Certificate[] c, String a) { }
    public X509Certificate[] getAcceptedIssuers() { return new X509Certificate[0]; }
}
