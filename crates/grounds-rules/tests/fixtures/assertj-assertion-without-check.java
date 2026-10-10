// grounds: test
import static org.assertj.core.api.Assertions.assertThat;
class ATest {
    void t(String s) {
        assertThat(s); // expect
        assertThat(s).as("desc"); // expect
        assertThat(s).isEqualTo("x");
        assertThat(s).as("desc").isNotNull();
    }
}
