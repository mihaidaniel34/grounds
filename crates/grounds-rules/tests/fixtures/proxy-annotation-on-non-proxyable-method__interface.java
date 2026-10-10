// grounds: no-findings
import org.springframework.transaction.annotation.Transactional;
// the project opts into JDK interface proxies, which intercept the final
// implementation of Service.value()
@org.springframework.transaction.annotation.EnableTransactionManagement(proxyTargetClass = false)
class Config {}
interface Service { int value(); }
class Implementation implements Service {
    @Transactional
    public final int value() { return 1; }
}
