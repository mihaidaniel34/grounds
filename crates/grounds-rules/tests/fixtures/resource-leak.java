import java.io.*;
import java.nio.file.*;
class A {
    void m(File f, Path p) throws IOException {
        InputStream a = new FileInputStream(f); // expect
        a.read();
        InputStream b = new FileInputStream(f);
        try { b.read(); } finally { b.close(); }
        try (InputStream c = new FileInputStream(f)) { c.read(); }
        BufferedReader r = Files.newBufferedReader(p); // expect
        r.readLine();
        ByteArrayOutputStream bos = new ByteArrayOutputStream();
        bos.write(1);
    }
    InputStream open(File f) throws IOException {
        InputStream in = new FileInputStream(f);
        return in;
    }
    Reader wrap(File f) throws IOException {
        InputStream in = new FileInputStream(f);
        return new InputStreamReader(in);
    }
}
