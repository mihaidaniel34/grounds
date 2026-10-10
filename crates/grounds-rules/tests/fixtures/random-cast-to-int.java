import java.util.Random;
class A {
    int m(Random r) {
        int a = (int) Math.random(); // expect
        int b = (int) r.nextDouble(); // expect
        int ok = (int) (Math.random() * 10);
        return a + b;
    }
}
