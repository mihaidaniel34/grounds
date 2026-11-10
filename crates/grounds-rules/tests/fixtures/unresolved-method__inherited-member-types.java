// The member types a class inherits come before its own name, which is in
// scope only one level out (JLS 6.4.1, 8.5); a local class keeps its member
// classes.
class Pane {
    public static class Item extends Pane {
        public static class Spec { public void hello() { } }
    }
}

class SubPane extends Pane {
    public static class Item extends Pane {
        // `Item` is the inherited Pane.Item, not SubPane.Item itself
        public static class Spec extends Item.Spec {
            void t() { hello(); }
            void u() { this.goodbye(); } // expect
        }
    }
}

class Holder {
    class Part { }

    void m() {
        class Local {
            class Part { void bar() { } }
            Part part = new Part();
            void g() { part.bar(); }
            void h() { part.baz(); } // expect
        }
        new Local().g();
    }
}

class Spot { void ok() { } }

class PrivateMembers {
    void m() {
        class Base { private class Spot { void bad() { } } }
        // Base.Spot is private, so not inherited: `Spot` is the top-level class
        class Sub extends Base {
            void test(Spot s) { s.ok(); }
            void wrong(Spot s) { s.bad(); } // expect
        }
    }
}
