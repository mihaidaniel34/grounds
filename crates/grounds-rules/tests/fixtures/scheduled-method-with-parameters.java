import org.springframework.scheduling.annotation.Scheduled;
class A {
    @Scheduled(fixedRate = 1000) public void tick(String x) {} // expect
    @Scheduled(fixedRate = 1000) public void ok() {}
}
