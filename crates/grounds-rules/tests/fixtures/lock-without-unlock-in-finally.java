import java.util.concurrent.locks.*;
class A {
    final Lock lock = new ReentrantLock();
    void bad() {
        lock.lock(); // expect
        work();
        lock.unlock();
    }
    void good() {
        lock.lock();
        try { work(); } finally { lock.unlock(); }
    }
    void acquire() { lock.lock(); }
    void work() {}
}
