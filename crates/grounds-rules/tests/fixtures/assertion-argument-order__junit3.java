// grounds: test
import static junit.framework.Assert.assertEquals;
class LegacyTest extends junit.framework.TestCase {
    // junit.framework.Assert is not on the classpath: the static import still
    // says the message comes first
    public void testCode(CodedException e) {
        int code = e.getCode();
        assertEquals("unexpected code " + code, 0, code);
        assertEquals("unexpected code", code, 0); // expect
    }
}
