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
class TryLocked {
    private volatile int count;
    private final java.util.concurrent.locks.ReentrantLock lock = new java.util.concurrent.locks.ReentrantLock();
    void inc() {
        if (lock.tryLock()) {
            try { count++; } finally { lock.unlock(); }
        }
    }
    void bail() {
        if (!lock.tryLock()) {
            return;
        }
        try { count++; } finally { lock.unlock(); }
    }
    void block() {
        synchronized (this) { count += 2; }
    }
    // the update runs whether or not the lock was taken
    void ignored() {
        boolean locked = lock.tryLock();
        count++; // expect
        if (locked) lock.unlock();
    }
    void missed() {
        if (!lock.tryLock()) {
            count++; // expect
            return;
        }
        lock.unlock();
    }
    // "Guarded by lock" in a comment is not evidence
    void commented() { count--; } // expect
}
