package app;
import org.springframework.stereotype.Service;
import org.springframework.beans.factory.annotation.Qualifier;
interface Store {}
@Service("primaryStore") class DbStore implements Store {}
@Service class Uses {
    Uses(@Qualifier("primarystore") Store s) {} // expect
}
@Service class Ok {
    Ok(@Qualifier("primaryStore") Store s) {}
}

// the project is an application: its context is complete
@org.springframework.boot.autoconfigure.SpringBootApplication class FixtureApp {}
