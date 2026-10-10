// grounds: test
import static org.junit.jupiter.api.Assertions.*;
import static org.assertj.core.api.Assertions.assertThat;
class ATest {
    void t(String a, String b, Integer n, Object o) {
        assertEquals(a, a); // expect
        assertThat(a).isEqualTo(a); // expect
        assertEquals(n, n); // expect
        assertSame(o, o); // expect
        assertEquals(a, b);
        assertEquals(1, 1);
        // reflexivity of a user-defined equals
        assertEquals(o, o);
        assertThat(o).isEqualTo(o);
    }
}
