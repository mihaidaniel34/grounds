import java.net.*;
import java.util.*;
class A {
    void m(URL a, URL b, URI u) {
        boolean e = a.equals(b); // expect
        int h = a.hashCode(); // expect
        Set<URL> s = new HashSet<URL>(); // expect
        Map<URL, String> m = new HashMap<>(); // expect
        Set<URI> ok = new HashSet<>();
        boolean f = u.equals(u);
    }
}
