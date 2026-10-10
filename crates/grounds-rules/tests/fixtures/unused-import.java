import java.util.List;
import java.util.Map; // expect
import java.util.Set;
import java.util.List; // expect
import java.lang.String; // expect
import java.io.File;
import static java.util.Collections.emptyList;
import static java.util.Collections.singletonList; // expect
import java.util.Deque;
import java.util.Queue;
import java.util.Optional;
import java.util.Iterator; // expect
import java.util.NavigableSet;
/**
 * See {@link Set}, {@link
 * Deque#push(Object)} and {@link java.util.Collections#addAll(Queue, Object...)};
 * {@code Iterator} is only code; {@linkplain NavigableSet sets}.
 * @see A#m(Optional)
 */
class A {
    List<String> l = emptyList();
    void m(File f) {}
}
