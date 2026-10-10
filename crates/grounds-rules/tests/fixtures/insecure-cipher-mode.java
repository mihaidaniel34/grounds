import javax.crypto.Cipher;
class A {
    void m() throws Exception {
        Cipher a = Cipher.getInstance("AES"); // expect
        Cipher b = Cipher.getInstance("AES/ECB/PKCS5Padding"); // expect
        Cipher c = Cipher.getInstance("RSA/ECB/PKCS1Padding"); // expect
        Cipher d = Cipher.getInstance("AES/CBC/PKCS5Padding"); // expect
        Cipher ok = Cipher.getInstance("AES/GCM/NoPadding");
        Cipher ok2 = Cipher.getInstance("RSA/ECB/OAEPWithSHA-256AndMGF1Padding");
    }
}
