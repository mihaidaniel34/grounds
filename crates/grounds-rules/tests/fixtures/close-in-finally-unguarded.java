import java.io.*;
class A {
    void m(File f) throws IOException {
        InputStream in = new FileInputStream(f);
        try { in.read(); } finally {
            in.close(); // expect
        }
        try (InputStream in2 = new FileInputStream(f)) { in2.read(); }
    }
}
