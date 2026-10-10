import org.springframework.transaction.annotation.Transactional;
import org.springframework.scheduling.annotation.Async;
class A {
    @Transactional private void save() {} // expect
    @Async public final void run() {} // expect
    @Transactional public void ok() {}
}
