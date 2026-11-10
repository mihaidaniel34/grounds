import java.io.File;
import java.util.concurrent.locks.Lock;
class A {
    void m(File f, Lock lock) {
        f.delete(); // expect
        f.mkdirs(); // expect
        lock.tryLock(); // expect
        if (!f.delete()) { }
        boolean ok = f.mkdir();
    }
}
class MockSetup {
    void t(java.util.concurrent.BlockingQueue<String> queue) {
        org.mockito.Mockito.verify(queue).offer("x");
    }
}
class Latches {
    private final java.util.concurrent.CountDownLatch latch = new java.util.concurrent.CountDownLatch(1);
    void polled() throws InterruptedException {
        while (latch.getCount() != 0) {
            latch.await(1, java.util.concurrent.TimeUnit.SECONDS);
        }
    }
    void checkedAfter() throws InterruptedException {
        latch.await(1, java.util.concurrent.TimeUnit.SECONDS);
        if (latch.getCount() != 0) throw new IllegalStateException("timed out");
    }
    void unchecked() throws InterruptedException {
        latch.await(1, java.util.concurrent.TimeUnit.SECONDS); // expect
    }
    void loopNotOnCount() throws InterruptedException {
        while (latch != null) {
            latch.await(1, java.util.concurrent.TimeUnit.SECONDS); // expect
        }
    }
    void countIgnored() throws InterruptedException {
        latch.await(1, java.util.concurrent.TimeUnit.SECONDS); // expect
        latch.getCount();
    }
}
class Queues {
    private final java.util.concurrent.BlockingQueue<Integer> unbounded = new java.util.concurrent.LinkedBlockingQueue<>();
    private final java.util.concurrent.BlockingQueue<Integer> bounded = new java.util.concurrent.LinkedBlockingQueue<>(10);
    private final java.util.concurrent.PriorityBlockingQueue<Integer> prio = new java.util.concurrent.PriorityBlockingQueue<>();
    void m(java.util.concurrent.BlockingQueue<Integer> param) {
        unbounded.offer(1);
        this.unbounded.offer(2);
        prio.offer(3);
        bounded.offer(4); // expect
        param.offer(5); // expect
    }
    void shadowed(java.util.concurrent.BlockingQueue<Integer> unbounded) {
        unbounded.offer(6); // expect
    }
}
class Directories {
    void createThenWrite(File file, byte[] data) throws java.io.IOException {
        // a missing directory makes the next write throw
        file.getParentFile().mkdirs();
        file.createNewFile();
        java.nio.file.Files.write(file.toPath(), data);
    }
    void createThenStream(File dir) throws java.io.IOException {
        dir.mkdirs();
        try (java.io.FileOutputStream out = new java.io.FileOutputStream(new File(dir, "a.bin"))) { out.write(1); }
    }
    void unrelatedWrite(File dir, File other, byte[] data) throws java.io.IOException {
        dir.mkdirs(); // expect
        java.nio.file.Files.write(other.toPath(), data);
    }
    void unrelatedData(File dir, File marker) throws java.io.IOException {
        dir.mkdirs(); // expect
        java.nio.file.Files.writeString(marker.toPath(), dir.getName());
    }
    void tempWithAttributes(File dir, java.nio.file.attribute.FileAttribute<?> attr) throws java.io.IOException {
        dir.mkdirs();
        java.nio.file.Files.createTempFile(dir.toPath(), "a", ".tmp", attr);
    }
    void nameAsData(File dir, File other, byte[] data) throws java.io.IOException {
        dir.mkdirs(); // expect
        java.nio.file.Files.write(new File(other, dir.getName()).toPath(), data);
    }
    void childOfDir(File dir, byte[] data) throws java.io.IOException {
        dir.mkdirs();
        java.nio.file.Files.write(new File(dir, "a.bin").toPath(), data);
    }
    void createOnly(File dir) {
        dir.mkdirs(); // expect
    }
    void createThenRead(File dir) {
        dir.mkdirs(); // expect
        dir.listFiles();
    }
}
