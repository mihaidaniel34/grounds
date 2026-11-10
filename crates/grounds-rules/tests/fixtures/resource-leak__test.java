// grounds: test
import java.io.*;
class ParserTest {
    static class Pool implements Closeable { public void close() { } int idle() { return 0; } }
    void inMemoryAndDoubles(String json) throws IOException {
        // a test's own closeables hold nothing outside the heap
        Pool pool = new Pool();
        pool.idle();
        PushbackReader reader = new PushbackReader(new StringReader(json));
        reader.read();
    }
    void fileHandle(File f) throws IOException {
        // still reported: a file handle left open
        InputStream in = new BufferedInputStream(new FileInputStream(f)); // expect
        in.read();
    }
    void scannerAndWriter(File f) throws IOException {
        // constructors that open the file they are given
        java.util.Scanner sc = new java.util.Scanner(f); // expect
        sc.nextLine();
        PrintWriter pw = new PrintWriter(f); // expect
        pw.println("x");
        java.util.Scanner text = new java.util.Scanner("in memory");
        text.next();
    }
    void lines(java.nio.file.Path p) throws IOException {
        java.util.stream.Stream<String> lines = java.nio.file.Files.lines(p); // expect
        lines.count();
    }
    void connection(javax.sql.DataSource ds) throws java.sql.SQLException {
        java.sql.Statement st = ds.getConnection().createStatement(); // expect
        st.execute("select 1");
    }
}
