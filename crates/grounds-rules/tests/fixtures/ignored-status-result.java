import java.io.File;
import java.util.concurrent.locks.Lock;
class A {
    void m(File f, Lock lock) {
        f.delete(); // expect
        f.mkdirs(); // expect
        lock.tryLock(); // expect
        if (!f.delete()) { }
        boolean ok = f.mkdir();
    }
}
