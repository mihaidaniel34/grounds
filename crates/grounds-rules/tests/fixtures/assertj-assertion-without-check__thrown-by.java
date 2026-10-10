// grounds: test; grounds: no-findings
import static org.assertj.core.api.Assertions.assertThatThrownBy;
class ATest {
    void t() {
        assertThatThrownBy(() -> { throw new IllegalArgumentException(); });
    }
}
