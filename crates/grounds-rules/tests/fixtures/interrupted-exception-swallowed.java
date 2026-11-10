class A {
    void m() {
        try { Thread.sleep(10); } catch (InterruptedException e) { } // expect
        try { Thread.sleep(10); } catch (InterruptedException e) { System.out.println("x"); } // expect
        try { Thread.sleep(10); } catch (InterruptedException e) { Thread.currentThread().interrupt(); }
        try { Thread.sleep(10); } catch (InterruptedException e) { throw new RuntimeException(e); }
    }
}
class FailingTest {
    void t() {
        try { Thread.sleep(1); } catch (InterruptedException e) { org.junit.jupiter.api.Assertions.fail(e); }
    }
}
class ConditionalFail {
    void t(boolean fatal) {
        try { Thread.sleep(1); } catch (InterruptedException e) { if (fatal) org.junit.jupiter.api.Assertions.fail(e); } // expect
    }
}
class Deferred {
    void retry() {
        boolean interrupted = false;
        try {
            for (int i = 0; i < 3; i++) {
                try { Thread.sleep(10); } catch (InterruptedException e) { interrupted = true; break; }
            }
        } finally {
            if (interrupted) {
                Thread.currentThread().interrupt();
            }
        }
    }
    void afterLoop() {
        boolean interrupted = false;
        while (!done()) {
            try { Thread.sleep(10); } catch (InterruptedException e) { interrupted = true; }
        }
        if (interrupted) Thread.currentThread().interrupt();
    }
    // the flag is recorded and then nothing restores the interrupt
    boolean dropped() {
        boolean interrupted = false;
        try { Thread.sleep(10); } catch (InterruptedException e) { interrupted = true; } // expect
        if (interrupted) System.out.println("stopped");
        return interrupted;
    }
    // interrupting another thread does not restore this one's flag
    void other(Thread worker) {
        boolean interrupted = false;
        try { Thread.sleep(10); } catch (InterruptedException e) { interrupted = true; } // expect
        if (interrupted) worker.interrupt();
    }
    void helper(Thread worker) {
        boolean interrupted = false;
        try { Thread.sleep(10); } catch (InterruptedException e) { interrupted = true; } // expect
        if (interrupted) interrupt(worker);
    }
    void alias() {
        Thread self = Thread.currentThread();
        boolean interrupted = false;
        try { Thread.sleep(10); } catch (InterruptedException e) { interrupted = true; }
        if (interrupted) self.interrupt();
    }
    void interrupt(Thread t) { t.interrupt(); }
    boolean done() { return true; }
}
