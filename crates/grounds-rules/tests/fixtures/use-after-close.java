import java.io.*;
class A {
    void m(File f) throws IOException {
        InputStream in = new FileInputStream(f);
        in.close();
        in.read(); // expect
    }
}
