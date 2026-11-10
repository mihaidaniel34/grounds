import java.io.*;
import java.nio.charset.StandardCharsets;
class A {
    void m(String s, byte[] b, InputStream in, File f) throws IOException {
        byte[] x = s.getBytes(); // expect
        String y = new String(b); // expect
        Reader r = new InputStreamReader(in); // expect
        Reader fr = new FileReader(f); // expect
        byte[] ok = s.getBytes(StandardCharsets.UTF_8);
        String ok2 = new String(b, StandardCharsets.UTF_8);
        Reader ok3 = new InputStreamReader(in, StandardCharsets.UTF_8);
        String ok4 = new String(b, "UTF-8");
    }
    // the charset comes from a call that does not resolve: it may well be one
    void unknown(byte[] b, File f, lib.Headers h) throws IOException {
        String s = new String(b, lib.Util.charset(h));
        Reader r = new FileReader(f, lib.Util.charset(h));
        String still = new String(b); // expect
    }
}
