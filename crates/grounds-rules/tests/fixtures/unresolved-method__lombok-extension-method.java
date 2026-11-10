// grounds: no-findings
import lombok.experimental.ExtensionMethod;

class Greetings {
    public static String shout(Greeting g) {
        return g.text().toUpperCase();
    }
}

class Greeting {
    String text() {
        return "hi";
    }
}

// @ExtensionMethod makes Greetings.shout callable on a Greeting in this file
@ExtensionMethod(Greetings.class)
class Use {
    String m(Greeting g) {
        return g.shout();
    }
}
