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
class MockSetup {
    void t(java.util.concurrent.BlockingQueue<String> queue) {
        org.mockito.Mockito.verify(queue).offer("x");
    }
}
