import java.util.*;
class A {
    void m(String s, Object o, List<String> l) {
        String a = (String) s; // expect
        String b = (String) o;
        int i = (int) 3; // expect
        long c = (long) i;
        print((Object) s);
        String d = (String) l.get(0); // expect
    }
    void print(Object o) {}
    void print(String s) {}
}
