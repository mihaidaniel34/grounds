import java.util.Iterator;
class A {
    void m(int x, Iterator<String> it) {
        if (x > 0); // expect
        for (int i = 0; i < 3; i++); // expect
        while (it.next() != null);
        if (x > 1) { }
    }
}
