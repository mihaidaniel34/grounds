import java.util.List;
class A {
    String m(List<String> items) {
        String s = "";
        for (String i : items) {
            s += i; // expect
        }
        String t = "";
        int n = 0;
        while (n < 3) {
            t = t + n; // expect
            n++;
        }
        for (String i : items) {
            String local = "";
            local += i;
        }
        return s + t;
    }
}
