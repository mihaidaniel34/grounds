// grounds: test
import static org.junit.jupiter.api.Assertions.assertEquals;
import static ext.Helper.assertEquals;

// two static imports of assertEquals, and an argument whose type is unknown:
// either may be called, and the helper may take (actual, expected)
class AmbiguousImport {
    @org.junit.jupiter.api.Test void test(ext.Value value) {
        assertEquals(value, "expected");
    }
}
// an inherited helper next to a JUnit static import: the helper hides it
class InheritedNextToImport extends ext.Assertions {
    @org.junit.jupiter.api.Test void test(int actual) {
        assertEquals(actual, 42);
    }
}
class ImportOnly {
    @org.junit.jupiter.api.Test void test(int actual) {
        org.junit.jupiter.api.Assertions.assertEquals(actual, 42); // expect
    }
}
// JUnit's inherited assertEquals hides a TestNG static import: JUnit's order
class InheritedJUnitOverImport extends org.junit.Assert {
    @org.junit.Test void test(int actual) {
        assertEquals(actual, 42); // expect
    }
}
