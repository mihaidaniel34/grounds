import java.util.Random;
class A {
    int m(Object key, Random r, int n) {
        int a = Math.abs(key.hashCode()) % n; // expect
        int b = Math.abs(r.nextInt()); // expect
        int ok = Math.floorMod(key.hashCode(), n);
        return Math.abs(n);
    }
}
