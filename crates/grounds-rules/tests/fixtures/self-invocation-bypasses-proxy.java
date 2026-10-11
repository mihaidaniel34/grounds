import org.springframework.stereotype.Service;
import org.springframework.transaction.annotation.Transactional;
@Service class A {
    public void outer() {
        save(); // expect
        this.save(); // expect
    }
    @Transactional public void save() {}
    @Transactional public void alsoTx() { save(); }
}
@org.springframework.stereotype.Service
class EvictingService {
    public void save(String x) { handleEvict(x); }
    @org.springframework.transaction.event.TransactionalEventListener
    public void handleEvict(String x) { }
}
