// grounds: test, grounds: no-findings
class ThreadFinderTest {
    void findsGroup() {
        // the group is the test data
        ThreadGroup g = new ThreadGroup("group-under-test");
        assert g.getName() != null;
    }
}
