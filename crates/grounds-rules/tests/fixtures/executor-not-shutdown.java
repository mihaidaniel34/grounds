import java.util.concurrent.*;
class A {
    void bad() {
        ExecutorService ex = Executors.newFixedThreadPool(2); // expect
        ex.submit(() -> {});
    }
    void good() {
        ExecutorService ex = Executors.newFixedThreadPool(2);
        ex.submit(() -> {});
        ex.shutdown();
    }
    ExecutorService escapes() {
        ExecutorService ex = Executors.newCachedThreadPool();
        return ex;
    }
}
