// grounds: test, grounds: no-findings
import java.io.*;
class BufferTest {
    void readsAll() throws IOException {
        // an in-memory stream fills the buffer; the assertion checks the content
        InputStream in = new ByteArrayInputStream(new byte[] {1, 2});
        byte[] buf = new byte[2];
        in.read(buf);
        assert buf[1] == 2;
    }
}
