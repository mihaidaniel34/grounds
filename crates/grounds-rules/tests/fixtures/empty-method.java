class A {
    void nothing() { } // expect
    void documented() {
        // intentionally left blank: hook
    }
    void ok() { System.out.println(); }
}
abstract class B { void hook() { } }
