// grounds: test
import static org.assertj.core.api.Assertions.assertThat;
import org.hamcrest.MatcherAssert;
import org.hamcrest.Matchers;
class ATest {
    void t(int[] ids) {
        MatcherAssert.assertThat("ids length", ids.length, Matchers.greaterThanOrEqualTo(1));
        assertThat(ids.length > 0); // expect
    }
}
