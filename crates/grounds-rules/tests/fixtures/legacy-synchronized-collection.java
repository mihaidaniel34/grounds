import java.util.*;
class A {
    void m() {
        Vector<String> v = new Vector<>(); // expect
        Hashtable<String, String> h = new Hashtable<>(); // expect
        StringBuffer sb = new StringBuffer(); // expect
        Stack<String> s = new Stack<>(); // expect
        List<String> ok = new ArrayList<>();
    }
}
