import java.util.List;
import java.util.regex.Pattern;
class A {
    private static final Pattern P = Pattern.compile("\\d+");
    void m(List<String> l) {
        for (String s : l) {
            if (s.matches("\\d+")) { } // expect
            Pattern p = Pattern.compile("x"); // expect
            String t = s.replaceAll("abc", "d");
            boolean ok = P.matcher(s).matches();
        }
        boolean outside = "1".matches("\\d+");
    }
}
