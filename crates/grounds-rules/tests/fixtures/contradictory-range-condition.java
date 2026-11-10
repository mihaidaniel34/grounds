import java.util.List;

class A {
    static final int MIN = 10;
    static final int MAX = 5;
    int state;

    boolean alwaysFalse(int x, long l, Integer boxed, char c, List<String> list, String s) {
        boolean r = x > 5 && x < 3; // expect
        r |= x == 1 && x == 2; // expect
        r |= 5 < x && 3 > x; // expect
        r |= x > 5 && l > 0 && x <= 5; // expect
        r |= (x >= 7) && (x != 0) && (x < 7); // expect
        r |= x == 3 && x != 3; // expect
        r |= x > 5 && x < 6; // expect
        r |= l >= 100L && l < 100L; // expect
        r |= boxed == 1 && boxed == 2; // expect
        r |= c == 'a' && c == 'b'; // expect
        r |= x > MIN && x < MAX; // expect
        r |= list.size() > 3 && list.size() < 2; // expect
        r |= s.length() == 0 && s.length() > 0; // expect
        r |= this.state == 1 && this.state == 2; // expect
        return r;
    }

    boolean alwaysTrue(int x, double d) {
        boolean r = x != 1 || x != 2; // expect
        r &= x >= 0 || x < 0; // expect
        r &= x < 10 || x > 5; // expect
        r &= d != 1 || d != 2; // expect
        return r;
    }

    boolean fine(int x, int y, double d, List<String> list, java.util.Iterator<Integer> it, java.io.InputStream in) throws Exception {
        boolean r = x > 3 && x < 5;
        r |= x >= 5 && x <= 5;
        r |= x > 5 && y < 3;
        r |= x < 3 || x > 5;
        r |= x == 1 || x == 2;
        r |= x != 1 && x != 2;
        r |= x > 5 || x < 3 && y > 0;
        r |= x > 5 && (y > 0 || x < 3);
        // a double has values between 5 and 6
        r |= d > 5 && d < 6;
        // false for NaN: a NaN test
        r |= d >= 0 || d < 0;
        // each call gives a new value
        r |= it.next() == 1 && it.next() == 2;
        r |= in.read() == 1 && in.read() == 2;
        // the operand changes between the tests
        r |= x == 1 && x++ == 1 && x == 2;
        r |= x == 1 && (x = y) == 2 && x == 2;
        r |= state == 1 && advance() && state == 2;
        r |= list.size() == 1 && list.add("x") && list.size() == 2;
        return r;
    }

    boolean advance() {
        state++;
        return true;
    }
}
