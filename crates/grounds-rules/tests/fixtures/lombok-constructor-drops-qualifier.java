package app;
import lombok.RequiredArgsConstructor;
import org.springframework.beans.factory.annotation.Qualifier;
import org.springframework.stereotype.Service;
interface Store {}
@Service @RequiredArgsConstructor
class Uses {
    @Qualifier("mem") private final Store store; // expect
    private final Store other;
}
@Service
class Explicit {
    @Qualifier("mem") private final Store store;
    Explicit(@Qualifier("mem") Store store) { this.store = store; }
}
