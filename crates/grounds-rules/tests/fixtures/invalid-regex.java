import java.util.regex.Pattern;
class A {
    void m(String s) {
        Pattern a = Pattern.compile("(abc"); // expect
        boolean b = s.matches("*.txt"); // expect
        String c = s.replaceAll("${user}", "x"); // expect
        String[] d = s.split("[a-z"); // expect
        Pattern ok = Pattern.compile("\\d+(\\.\\d+)?");
        Pattern lit = Pattern.compile("(abc", Pattern.LITERAL);
        boolean ok2 = s.matches("[a-z]{2,4}");
    }
}
