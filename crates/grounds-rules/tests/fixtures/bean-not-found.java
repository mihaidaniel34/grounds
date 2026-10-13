package app;
import org.springframework.stereotype.Service;
interface Store {}
interface Mailer {}
@Service class DbStore implements Store {}
@Service class Uses {
    Uses(Store s, Mailer m) {} // expect
}
@Service class Optional1 {
    Optional1(java.util.Optional<Mailer> m, java.util.List<Mailer> all) {}
}

// the project is an application: its context is complete
@org.springframework.boot.autoconfigure.SpringBootApplication class FixtureApp {}
