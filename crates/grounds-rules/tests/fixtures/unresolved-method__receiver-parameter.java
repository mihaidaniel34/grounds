import java.lang.annotation.ElementType;
import java.lang.annotation.Target;

// An explicit receiver parameter carries type annotations and is not a
// parameter (JLS 8.4): `ok` takes one argument, `Inner(...)` one.
@Target(ElementType.TYPE_USE)
@interface Initialized {}

class ReceiverCase {
    boolean ok(@Initialized ReceiverCase this, int x) { return x > 0; }
    boolean plain(ReceiverCase this, int x) { return x > 0; }
    boolean use() { return ok(1) && this.ok(2) && plain(3); }
    boolean wrong() { return this.ok(); } // expect
    boolean wrong2() { return this.ok(1, 2); } // expect

    class Inner {
        Inner(@Initialized ReceiverCase ReceiverCase.this, int y) {}
        void take(@Initialized Inner this) {}
    }

    void make() {
        Inner i = new Inner(1);
        i.take();
        i.take(i); // expect
    }
}
