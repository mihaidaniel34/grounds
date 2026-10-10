// grounds: no-findings
import org.springframework.transaction.annotation.Transactional;
// a JDK interface proxy intercepts the final implementation of Service.value()
interface Service { int value(); }
class Implementation implements Service {
    @Transactional
    public final int value() { return 1; }
}
