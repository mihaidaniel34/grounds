// grounds: test
import static org.mockito.Mockito.*;
import java.util.List;
class ATest {
    void t() {
        List<String> m = mock(List.class);
        when(m.get(0)); // expect
        verify(m); // expect
        when(m.get(0)).thenReturn("x");
        verify(m).get(0);
    }
}
