// grounds: test, grounds: no-findings
class StubChecker implements Cloneable {
    @Override public StubChecker clone() { return new StubChecker(); }
}
