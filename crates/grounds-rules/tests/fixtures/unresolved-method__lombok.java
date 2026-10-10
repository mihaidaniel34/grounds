import lombok.Builder;
import lombok.Data;
import lombok.Getter;
import lombok.experimental.SuperBuilder;
import lombok.experimental.UtilityClass;
import java.util.List;

@UtilityClass
class Params {
    String serialize(List<String> values) { return String.join(",", values); }
}

class Request {
    private final String id;
    private final long timeout;
    @Builder
    private Request(String id, long timeout) { this.id = id; this.timeout = timeout; }
}

@Data(staticConstructor = "of")
class SaveResult {
    private final int count;
}

@Data
@Builder
class Event {
    private String name;
    @Builder.Default
    private final long ts = System.currentTimeMillis();
}

@Data
@SuperBuilder
class BaseConfig {
    protected List<String> components;
}

@Data
@SuperBuilder
class PdfConfig extends BaseConfig {
    private String title;
}

class Pool {
    @Getter
    private static final List<String> pool = List.of();
}

class Use {
    void m() {
        String s = Params.serialize(List.of("a"));
        Request r = Request.builder().id("x").timeout(1).build();
        SaveResult res = SaveResult.of(3);
        Event e = Event.builder().name("n").ts(5).build();
        Event all = new Event("n", 5);
        PdfConfig c = PdfConfig.builder().components(List.of()).title("t").build();
        List<String> p = Pool.getPool();
        Request.builder().nope(); // expect
    }
}
