// grounds: test, grounds: no-findings
class WatchJobTest {
    void runsOnePass(Thread job) {
        // the task body runs here, so the assertions that follow see its effects
        job.run();
        assert !job.isAlive();
    }
}
