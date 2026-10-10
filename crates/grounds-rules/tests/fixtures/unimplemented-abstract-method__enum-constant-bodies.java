import java.util.function.Function;
// each constant implements apply() in its own body
enum Operator implements Function<String, String> {
    UPPER { public String apply(String s) { return s.toUpperCase(); } },
    LOWER { public String apply(String s) { return s.toLowerCase(); } };
}
enum Broken implements Runnable { A, B } // expect
