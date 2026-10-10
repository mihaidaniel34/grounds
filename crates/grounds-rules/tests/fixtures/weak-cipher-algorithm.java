import javax.crypto.*;
class A {
    void m() throws Exception {
        Cipher a = Cipher.getInstance("DES/CBC/PKCS5Padding"); // expect
        KeyGenerator k = KeyGenerator.getInstance("RC4"); // expect
        Cipher ok = Cipher.getInstance("AES/GCM/NoPadding");
    }
}
