import lombok.AllArgsConstructor;
import lombok.RequiredArgsConstructor;
import java.util.Map;

// the generated `of(String, String, Object)` sits beside hand-written
// overloads, one of them with the same number of parameters
@AllArgsConstructor(staticName = "of")
class Message {
    private String type;
    private String text;
    private Object data;

    static Message of(Session session) { return of("open", "opened", session); }
    static Message of(Session session, Topic topic, int qos) { return of("subscribe", topic.name, session); }
}

@RequiredArgsConstructor(staticName = "create")
class Pair {
    private final String left;
    private final String right;

    static Pair create(String both) { return new Pair(both, both); }
}

// a hand-written factory taking another `Widget` than the field's
@AllArgsConstructor(staticName = "of")
class Holder {
    private Left.Widget widget;

    static Holder of(Right.Widget widget) { return of(new Left.Widget()); }
}

class Left { static class Widget {} }
class Right { static class Widget {} }

class Session {}
class Topic { String name; }

class Use {
    void m(Map<String, Object> data, Session s, Topic t) {
        Message a = Message.of("connection", "connected", data);
        Message b = Message.of(s);
        Message c = Message.of(s, t, 1);
        Pair p = Pair.create("l", "r");
        Pair q = Pair.create("both");
        Holder h = Holder.of(new Left.Widget());
        Holder k = Holder.of(new Right.Widget());
        Message.of("a", "b"); // expect
        Pair.create(1, 2, 3); // expect
    }
}
