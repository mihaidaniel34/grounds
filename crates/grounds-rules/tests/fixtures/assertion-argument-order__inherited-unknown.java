// grounds: test; grounds: no-findings
// assertEquals inherited from a class that is not on the classpath: it may
// take (actual, expected), so the order is not judged
class OrderTest extends ext.Assertions {
    void test(int actual) { assertEquals(actual, 42); }
}
