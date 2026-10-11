class A {
    private volatile int count;
    private int plain;
    void m() {
        count++; // expect
        count += 2; // expect
        plain++;
        count = 5;
    }
    synchronized void n() { count++; }
}
class LockedCounter {
    private volatile int count;
    private final java.util.concurrent.locks.ReentrantLock lock = new java.util.concurrent.locks.ReentrantLock();
    void inc() { lock.lock(); try { count++; } finally { lock.unlock(); } }
}
class UnlockedBetween {
    private volatile int count;
    private final java.util.concurrent.locks.ReentrantLock lock = new java.util.concurrent.locks.ReentrantLock();
    void m() { lock.lock(); lock.unlock(); count++; lock.lock(); lock.unlock(); } // expect
}
class ConditionalLock {
    private volatile int count;
    private final java.util.concurrent.locks.ReentrantLock lock = new java.util.concurrent.locks.ReentrantLock();
    void m(boolean guarded) {
        if (guarded) lock.lock();
        try { count++; } finally { if (guarded) lock.unlock(); } // expect
    }
}
