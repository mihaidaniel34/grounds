// grounds: test, grounds: no-findings
// stubs that return null to exercise the caller
class NullStubs {
    Object unprintable = new Object() {
        @Override public String toString() { return null; }
    };
    static class Uncloneable implements Cloneable {
        @Override public Object clone() { return null; }
    }
}
