import lombok.Builder;
import lombok.Getter;
import lombok.Setter;
import lombok.Singular;
import lombok.Value;
import lombok.experimental.Delegate;
import lombok.experimental.StandardException;
import lombok.experimental.SuperBuilder;
import lombok.experimental.Tolerate;
import java.util.ArrayList;
import java.util.Date;
import java.util.List;
import java.util.Map;
import java.util.Set;

@Builder(setterPrefix = "with", buildMethodName = "create")
class Mail {
    private String subject;
    @Singular
    private List<String> recipients;
    @Singular
    private Map<String, String> headers;
    @Singular("child")
    private Set<Long> children;
    @Singular
    private List<String> addresses;
    @Singular
    private List<String> boxes;
}

class Box<T> {
    private final T value;

    @Builder
    Box(T value) {
        this.value = value;
    }

    @Builder(builderMethodName = "pairBuilder")
    static <X> Map.Entry<X, X> pair(X left, X right) {
        return null;
    }
}

@Builder(builderMethodName = "")
class Hidden {
    private int n;
}

@SuperBuilder
class Base {
    private String id;
}

class Derived extends Base {
    Derived(BaseBuilder<?, ?> b) {
        super(b);
    }
}

@Value(staticConstructor = "of")
class Money {
    long cents;
}

@StandardException
class QuotaExceeded extends RuntimeException {
}

class Event {
    @Setter
    private Date at;

    @Tolerate
    public void setAt(String iso) {
    }
}

// not modelled: the class, and classes extending it, are open
class Members {
    @Delegate
    private final List<String> items = new ArrayList<>();
}

class MoreMembers extends Members {
}

class Cached {
    @Getter(lazy = true)
    private final String value = "v";
}

class Use {
    void m(Event e, MoreMembers mm, Cached c) {
        Mail mail = Mail.builder().withSubject("s").withRecipient("a").withRecipients(List.of("b")).clearRecipients()
                .withHeader("k", "v").withHeaders(Map.of()).clearHeaders().withChild(1L).withAddress("x").withBox("y").create();
        Mail.builder().subject("s"); // expect
        Mail.builder().withRecipient("a").build(); // expect
        Mail.builder().withChildren(Set.of(2L)).withChilds(Set.of()); // expect
        Box<String> box = Box.<String>builder().value("v").build();
        Map.Entry<Integer, Integer> p = Box.<Integer>pairBuilder().left(1).right(2).build();
        Box.builder().weight(3); // expect
        Hidden.builder(); // expect
        Money m = Money.of(5);
        QuotaExceeded q1 = new QuotaExceeded("msg", new IllegalStateException());
        QuotaExceeded q2 = new QuotaExceeded(new IllegalStateException());
        e.setAt(new Date());
        e.setAt("2026-01-01");
        mm.size();
        mm.isEmpty();
        c.getValue();
    }
}
