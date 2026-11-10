// grounds: test
class WorkerTest {
    private final java.util.concurrent.CountDownLatch done = new java.util.concurrent.CountDownLatch(1);
    void waitsBounded(java.util.concurrent.ExecutorService pool) throws InterruptedException {
        // a bounded wait in a test: the assertions after it notice a timeout
        done.await(5, java.util.concurrent.TimeUnit.SECONDS);
        pool.shutdown();
        pool.awaitTermination(30, java.util.concurrent.TimeUnit.SECONDS);
    }
    void cleanup(java.io.File stale) {
        stale.delete(); // expect
    }
}
