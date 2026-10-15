// Inherited from a JDK class: List's SequencedCollection methods (Java 21).
// The fixtures run at release 21, which is the running JDK's own release on
// JDK 21 (the classes come from the runtime image).
import java.util.concurrent.CopyOnWriteArrayList;

class Meter extends CopyOnWriteArrayList<String> {}

class MeterUser {
    void m(Meter meter) {
        meter.getFirst();
        meter.getLast();
        meter.reversed();
        meter.nope(); // expect
    }
}
