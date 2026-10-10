import java.security.MessageDigest;
class A {
    void m() throws Exception {
        MessageDigest a = MessageDigest.getInstance("MD5"); // expect
        MessageDigest b = MessageDigest.getInstance("SHA-1"); // expect
        MessageDigest c = MessageDigest.getInstance("SHA-256");
    }
}
