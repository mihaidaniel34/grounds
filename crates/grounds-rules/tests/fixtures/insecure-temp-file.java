import java.io.File;
import java.nio.file.Files;
class A {
    void m() throws Exception {
        File a = File.createTempFile("x", ".tmp"); // expect
        File b = new File("/tmp/data.txt"); // expect
        Files.createTempFile("x", ".tmp");
    }
}
