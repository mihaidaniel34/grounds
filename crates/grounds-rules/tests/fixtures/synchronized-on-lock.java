import java.util.concurrent.locks.*;
class A {
    final Lock lock = new ReentrantLock();
    final Object mon = new Object();
    void m() {
        synchronized (lock) { } // expect
        synchronized (mon) { }
    }
}
