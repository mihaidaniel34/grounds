// grounds: no-findings
import javax.net.ssl.*;
class Checking implements HostnameVerifier {
    HostnameVerifier delegate;
    public boolean verify(String host, SSLSession session) {
        return delegate.verify(host, session) == true;
    }
}
