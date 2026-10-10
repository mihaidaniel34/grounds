// grounds: no-findings
import org.springframework.beans.factory.annotation.Autowired;
// Spring injects final fields reflectively
class A {
    @Autowired private final Object dep = null;
}
