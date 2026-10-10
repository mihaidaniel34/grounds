import java.util.Random;
class A {
    private final Random rnd = new Random();
    int m() {
        int a = new Random().nextInt(10); // expect
        return a + rnd.nextInt();
    }
}
