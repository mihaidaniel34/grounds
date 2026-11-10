// A pattern variable is in scope only where its match is definitely true
// (JLS 6.3.1, 6.3.2); elsewhere the name is the field again.
class LVar {
    Object obj() { return null; }
}

class UnitType {
    boolean controllable;
    void stop() { }
}

class PatternScope {
    LVar type;

    void run() {
        if (type.obj() instanceof UnitType type && type.controllable) {
            type.stop();
        } else if (type.obj() == null) {
            type.stop(); // expect
        }
        type.obj();
    }

    void negated(Object o) {
        if (!(o instanceof UnitType type)) {
            type.obj();
            return;
        }
        type.stop();
        type.obj(); // expect
    }

    void loop(Object o) {
        while (!(o instanceof UnitType type)) {
            o = type.obj();
        }
        type.stop();
    }

    void loopWithSwitch(Object o, int n) {
        while (!(o instanceof UnitType type)) {
            switch (n) { default: break; }
            o = new UnitType();
        }
        type.stop();
    }

    void elseLoops(Object o) {
        if (!(o instanceof UnitType type)) {
            while (true) { }
        }
        type.stop();
    }
}

class Good { void ok() { } }
class Bad { void other() { } }

class CompletionRules {
    Bad s;

    void tryFinally(Object o) {
        if (!(o instanceof Good s)) {
            try { throw new IllegalArgumentException(); } finally { System.out.println("bad"); }
        }
        s.ok();
    }

    void nestedLoopBreak(Object o) {
        while (!(o instanceof Good s)) {
            for (;;) { break; }
            o = new Good();
        }
        s.ok();
    }

    void completes(Object o) {
        if (!(o instanceof Good s)) {
            try { System.out.println("x"); } finally { System.out.println("y"); }
        }
        s.other();
        s.ok(); // expect
    }
}
