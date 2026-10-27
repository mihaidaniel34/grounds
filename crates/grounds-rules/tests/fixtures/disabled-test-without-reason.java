// grounds: test
import org.junit.jupiter.api.*;
class ATest {
    @Disabled @Test void a() { } // expect
    @Disabled("flaky on CI, see JIRA-123") @Test void b() { }
    @org.junit.Ignore @Test void c() { } // expect
}
record Book(String title, @GraphQl.Ignore String internal) {
}

