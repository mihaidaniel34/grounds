import java.util.Objects;

// a record whose last component is variable arity: its canonical
// constructor is varargs, also when the record declares it compactly
record Compact(String name, String field, Object... arguments) {
    Compact {
        Objects.requireNonNull(name);
    }
}

record Implicit(String name, Object... arguments) {}

class Uses {
    Object a() { return new Compact("count", "age"); }
    Object b() { return new Compact("between", "age", 1, 2); }
    Object c() { return new Implicit("x"); }
    Object d() { return new Compact("x"); } // expect
}
