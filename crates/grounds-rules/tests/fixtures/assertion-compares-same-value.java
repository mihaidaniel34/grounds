// grounds: test
import static org.junit.jupiter.api.Assertions.*;
import static org.assertj.core.api.Assertions.assertThat;
class ATest {
    void t(String a, String b) {
        assertEquals(a, a); // expect
        assertThat(a).isEqualTo(a); // expect
        assertEquals(a, b);
        assertEquals(1, 1);
    }
}
