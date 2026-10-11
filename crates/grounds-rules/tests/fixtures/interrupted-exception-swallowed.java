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
