// In `outer.new Builder(..)`, `Builder` is a member type of the qualifier's
// type (JLS 15.9.1), not the `Builder` in scope.
class Engine {
    public class Builder {
        public Builder(int x) { }
        public Builder permitted(String... s) { return this; }
    }
}

class UseEngine {
    static class Builder { }

    Engine.Builder make(Engine o) {
        return o.new Builder(1).permitted("js");
    }

    Engine.Builder wrong(Engine o) {
        return o.new Builder(); // expect
    }

    Builder own() { return new Builder(); }
}
