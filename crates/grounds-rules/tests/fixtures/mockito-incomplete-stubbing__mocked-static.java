// grounds: test; grounds: no-findings
import org.mockito.MockedStatic;
import org.mockito.Mockito;
class Sample { static void call() {} }
class ATest {
    void t() {
        try (MockedStatic<Sample> mocked = Mockito.mockStatic(Sample.class)) {
            Sample.call();
            mocked.verify(Sample::call);
        }
    }
}
