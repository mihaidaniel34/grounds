// Inside a class body, a field the class inherits shadows a local variable
// of the enclosing method (JLS 6.4.1).
class Layout {
    void all() { }
}

class Spec {
    int length() { return 0; }
}

class Plan {
    protected final Spec liveAndDown;
    private final Layout hidden = null;
    Plan(Spec s) { liveAndDown = s; }
}

class ShadowUse {
    Plan f() {
        Layout liveAndDown = new Layout();
        Layout hidden = new Layout();
        Layout other = new Layout();
        liveAndDown.all();
        return new Plan(new Spec()) {
            int g() { return liveAndDown.length(); }
            void h() { hidden.all(); other.all(); }
            int k() { return other.length(); } // expect
            void l() { liveAndDown.all(); } // expect
        };
    }
}
