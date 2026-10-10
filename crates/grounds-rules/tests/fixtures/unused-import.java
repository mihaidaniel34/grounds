import java.util.List;
import java.util.Map; // expect
import java.util.Set;
import java.util.List; // expect
import java.lang.String; // expect
import java.io.File;
import static java.util.Collections.emptyList;
import static java.util.Collections.singletonList; // expect
/** See {@link Set}. */
class A {
    List<String> l = emptyList();
    void m(File f) {}
}
