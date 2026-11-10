// grounds: test, grounds: no-findings
class CountingStub {
    volatile int invocations;
    void call() { invocations++; }
}
