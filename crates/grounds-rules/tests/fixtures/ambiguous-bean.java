package app;
import org.springframework.stereotype.Service;
import org.springframework.context.annotation.Primary;
import org.springframework.beans.factory.annotation.Qualifier;
interface Store {}
@Service class DbStore implements Store {}
@Service class MemStore implements Store {}
@Service class Uses {
    Uses(Store s) {} // expect
}
@Service class ByName {
    ByName(Store memStore) {}
}
@Service class Qualified {
    Qualified(@Qualifier("dbStore") Store s) {}
}
interface Clock {}
@Service @Primary class SystemClock implements Clock {}
@Service class FakeClock implements Clock {}
@Service class UsesClock { UsesClock(Clock c) {} }
@Service class Lists { Lists(java.util.List<Store> all) {} }

// the project is an application: its context is complete
@org.springframework.boot.autoconfigure.SpringBootApplication class FixtureApp {}
