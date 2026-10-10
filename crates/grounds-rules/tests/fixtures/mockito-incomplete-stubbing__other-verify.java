// grounds: test; grounds: no-findings
import static org.mockito.Mockito.*;
import reactor.test.StepVerifier;
class ATest {
    void reactor(reactor.core.publisher.Mono<String> m) {
        m.as(StepVerifier::create).expectNext("x").verify();
    }
    void jwt(com.auth0.jwt.JWTVerifier verifier, String token) {
        verifier.verify(token);
    }
}
