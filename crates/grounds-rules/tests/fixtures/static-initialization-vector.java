import javax.crypto.spec.*;
class A {
    private static final byte[] IV = new byte[16];
    void m(byte[] random) {
        IvParameterSpec a = new IvParameterSpec(new byte[] {1, 2, 3, 4}); // expect
        IvParameterSpec b = new IvParameterSpec("0123456789abcdef".getBytes()); // expect
        IvParameterSpec c = new IvParameterSpec(IV); // expect
        IvParameterSpec ok = new IvParameterSpec(random);
    }
}
