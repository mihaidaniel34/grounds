import java.util.concurrent.locks.*;
class A {
    final Lock lock = new ReentrantLock();
    final Condition c = lock.newCondition();
    void m() throws InterruptedException {
        c.wait(); // expect
        c.notify(); // expect
        c.await();
    }
}
