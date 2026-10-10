import org.springframework.scheduling.annotation.Async;
import java.util.concurrent.CompletableFuture;
class A {
    @Async public String compute() { return "x"; } // expect
    @Async public void fire() {}
    @Async public CompletableFuture<String> ok() { return null; }
}
