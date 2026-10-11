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
class Frames extends java.io.InputStream {
    public int read() { return -1; }
    boolean read(String frame) { return frame.isEmpty(); }
    void go() { read("hello"); }
}
