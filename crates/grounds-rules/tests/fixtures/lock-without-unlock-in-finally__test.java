// grounds: test, grounds: no-findings
import java.util.concurrent.locks.*;
class ReentrancyTest {
    void reenters(Lock lock) {
        // the lock is the thing under test
        lock.lock();
        lock.lock();
        lock.unlock();
        lock.unlock();
    }
}
