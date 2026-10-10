import java.io.File;
import java.nio.file.attribute.*;
import java.util.*;
class A {
    void m(File f, Set<PosixFilePermission> perms) {
        Set<PosixFilePermission> p = PosixFilePermissions.fromString("rwxrwxrwx"); // expect
        f.setWritable(true, false); // expect
        perms.add(PosixFilePermission.OTHERS_WRITE); // expect
        Set<PosixFilePermission> ok = PosixFilePermissions.fromString("rw-r-----");
    }
}
