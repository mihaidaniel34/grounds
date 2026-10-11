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
class LockAliases {
    private final java.util.concurrent.locks.ReentrantLock lock = new java.util.concurrent.locks.ReentrantLock();
    void a() { this.lock.lock(); try { work(); } finally { lock.unlock(); } }
    void b() { lock.lock(); try { work(); } catch (RuntimeException e) { lock.unlock(); throw e; } }
    void work() {}
}
