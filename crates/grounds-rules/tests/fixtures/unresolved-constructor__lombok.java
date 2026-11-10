import lombok.Value;
import lombok.experimental.StandardException;
import lombok.experimental.SuperBuilder;

@SuperBuilder
class Base {
    private String id;
}

// the protected constructor @SuperBuilder adds, taking the builder, is
// the only one
class Derived extends Base {
    Derived(BaseBuilder<?, ?> b) {
        super(b);
    }

    static Base copy(BaseBuilder<?, ?> b) {
        return new Base(b);
    }

    static Base empty() {
        return new Base(); // expect
    }
}

@StandardException
class QuotaExceeded extends RuntimeException {
}

@Value
class Point {
    int x;
    int y;
}

class Use {
    void m() {
        Object a = new QuotaExceeded();
        Object b = new QuotaExceeded("msg");
        Object c = new QuotaExceeded(new IllegalStateException());
        Object d = new QuotaExceeded("msg", new IllegalStateException());
        Object e = new QuotaExceeded("msg", new IllegalStateException(), 3); // expect
        Object f = new Point(1, 2);
        Object g = new Point(1); // expect
    }
}
