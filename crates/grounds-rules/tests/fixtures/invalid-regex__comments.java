// grounds: no-findings
import java.util.regex.Pattern;
class A {
    Pattern p = Pattern.compile("(?x)a # an unmatched ( in a comment\n b");
    Pattern q = Pattern.compile("a # ( comment", Pattern.COMMENTS);
}
