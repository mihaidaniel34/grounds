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
    void connection(javax.sql.DataSource ds) throws java.sql.SQLException {
        java.sql.Statement st = ds.getConnection().createStatement(); // expect
        st.execute("select 1");
    }
}
