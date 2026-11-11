// grounds: test
import kotlin.test.AssertionsKt;
class KotlinTestFromJava {
    // kotlin.test is not in the index: the import and the qualified name say
    // whose method it is
    @org.junit.jupiter.api.Test void t() {
        int r = 1;
        AssertionsKt.assertEquals(r, 42, null); // expect
        kotlin.test.AssertionsKt.assertEquals(r, 43, null); // expect
        AssertionsKt.assertEquals(42, r, null);
    }
}
