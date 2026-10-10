import java.io.File;
class A {
    void m(String path) {
        String[] parts = path.split(File.separator); // expect
        String[] ok = path.split(java.util.regex.Pattern.quote(File.separator));
    }
}
