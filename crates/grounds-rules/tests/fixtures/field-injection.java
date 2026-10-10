import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.stereotype.Service;
@Service class A {
    @Autowired private Object dep; // expect
    @jakarta.inject.Inject private Object other; // expect
    private final Object ok;
    A(Object ok) { this.ok = ok; }
}
