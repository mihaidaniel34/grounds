import org.slf4j.Logger;
import org.slf4j.LoggerFactory;
class A {
    static final int maxCount = 10; // expect
    static final String defaultName = "x"; // expect
    static final int MAX_COUNT = 10;
    private static final Logger log = LoggerFactory.getLogger(A.class);
    private static final long serialVersionUID = 1L;
    private static final java.util.regex.Pattern whitespace = java.util.regex.Pattern.compile("\\s+"); // expect
    private static final java.util.List<String> names = new java.util.ArrayList<>();
    private static final Mode defaultMode = Mode.ON; // expect
}
enum Mode { ON, OFF }
