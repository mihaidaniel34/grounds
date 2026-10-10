import java.security.KeyPairGenerator;
import javax.crypto.KeyGenerator;
class A {
    void m(KeyPairGenerator kpg, KeyGenerator kg) {
        kpg.initialize(1024); // expect
        kg.init(64); // expect
        kpg.initialize(2048);
        kpg.initialize(256);
    }
}
