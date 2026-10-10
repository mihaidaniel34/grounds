import java.security.SecureRandom;
class A {
    void m(byte[] seed) {
        SecureRandom a = new SecureRandom("fixed".getBytes()); // expect
        SecureRandom b = new SecureRandom();
        b.setSeed(42L); // expect
        b.setSeed(seed);
    }
}
