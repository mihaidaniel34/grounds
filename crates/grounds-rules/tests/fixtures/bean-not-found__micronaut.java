package mn2;
import jakarta.inject.*;
interface Engine {}
interface Wheel {}
class Tire implements Wheel { @Inject Tire() {} }
@Singleton class Car {
    @Inject Engine defaulted = null;
    @Inject Car(Wheel w, Engine e) {} // expect
}
