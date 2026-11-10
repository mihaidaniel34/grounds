// Member types a class inherits shadow those of the enclosing classes, also
// in anonymous and local classes (JLS 6.4.1, 8.5).
abstract class Call {
    static abstract class Listener { }
    abstract void start(Listener l);
}

abstract class Resolver {
    static abstract class Listener { }
    abstract void begin(Listener l);
}

class Channel extends Resolver {
    @Override
    void begin(Listener l) { }

    Call anonymous() {
        return new Call() {
            @Override
            void start(Listener l) { }
        };
    }

    Call local() {
        class LocalCall extends Call {
            @Override
            void start(Listener l) { }
        }
        return new LocalCall();
    }

    Call wrong() {
        return new Call() {
            @Override void start(Listener l) { }
            @Override void start(Resolver.Listener l) { } // expect
        };
    }

    @Override void begin(Call.Listener l) { } // expect
}
