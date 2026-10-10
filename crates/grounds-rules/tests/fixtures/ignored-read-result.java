import java.io.*;
class A {
    void m(InputStream in, Reader r, byte[] buf, char[] cs) throws IOException {
        in.read(buf); // expect
        in.skip(10); // expect
        r.read(cs); // expect
        int n = in.read(buf);
        in.read();
    }
}
