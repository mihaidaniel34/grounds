// An anonymous class created with the diamond (`new PostProcessor<>() {...}`)
// has the type arguments javac infers from the context (JLS 15.9.3), which
// grounds does not infer: a method may override through them, directly or
// through a method type variable bounded by them (spring-security's
// `ObjectPostProcessor<T>` with `<O extends T> O postProcess(O)`). Only what
// overrides nothing whatever they are is reported: no supertype method of
// the name and parameter count, or a parameter that differs without
// involving them. With the arguments written, they are known as before:
// a type variable of an enclosing class or method among them erases to its
// bound there (lucene's `new PriorityQueue<T>(n) { lessThan(Sub, Sub) }` in
// a class `<T extends Sub>`).
interface PostProcessor<T> { <O extends T> O postProcess(O object); }

interface Handler<T> { void handle(T value); }

interface Sink<T> { void put(String key, T value); }

interface Manager { }

class Config {
    PostProcessor<Manager> processor() {
        return new PostProcessor<>() {
            @Override public Manager postProcess(Manager object) { return object; }
        };
    }

    Handler<String> handler() {
        return new Handler<>() {
            @Override public void handle(String value) { }
        };
    }

    Sink<Manager> sink() {
        return new Sink<>() {
            @Override public void put(String key, Manager value) { }
        };
    }

    Handler<String> misspelled() {
        return new Handler<>() {
            @Override public void handle(String value) { }
            @Override public void handel(String value) { } // expect
        };
    }

    PostProcessor<Manager> moreParameters() {
        return new PostProcessor<>() {
            @Override public Manager postProcess(Manager object) { return object; }
            @Override public Manager postProcess(Manager object, int times) { return object; } // expect
        };
    }

    Sink<Manager> otherKey() {
        return new Sink<>() {
            @Override public void put(String key, Manager value) { }
            @Override public void put(Integer key, Manager value) { } // expect
        };
    }

    PostProcessor<Manager> explicit() {
        return new PostProcessor<Manager>() {
            @Override public Manager postProcess(Manager object) { return object; }
            @Override public String postProcess(String object) { return object; } // expect
        };
    }
}

abstract class Queue<T> { protected abstract boolean lessThan(T a, T b); }

class Sub { int doc; }

class Merger<T extends Sub> {
    Queue<T> queue() {
        return new Queue<T>() {
            @Override protected boolean lessThan(Sub a, Sub b) { return a.doc < b.doc; }
        };
    }

    <S extends Sub> Queue<S> local() {
        return new Queue<S>() {
            @Override protected boolean lessThan(Sub a, Sub b) { return a.doc < b.doc; }
            @Override protected boolean lessThan(Sub a) { return false; } // expect
        };
    }
}
