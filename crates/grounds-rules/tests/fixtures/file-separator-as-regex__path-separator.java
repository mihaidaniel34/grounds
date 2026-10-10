import java.io.File;
class A {
    String[] m(String paths) { return paths.split(File.pathSeparator); }
    String[] n(String path) { return path.split(File.separator); } // expect
}
