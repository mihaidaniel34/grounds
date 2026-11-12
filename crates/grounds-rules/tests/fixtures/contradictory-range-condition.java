import java.util.List;

enum Color { RED, GREEN }

class A {
    static final int MIN = 10;
    static final int MAX = 5;
    int state;
    static final String FIXED = "abc";
    static final int[] ARRAY = {1, 2};
    final String fixed = "abc";
    volatile String text;
    volatile int[] array;
    volatile int value;

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
        r |= s.length() == 0 && s.length() > 0; // expect
        r |= this.fixed.length() == 1 && this.fixed.length() == 2; // expect
        r |= Color.RED.ordinal() > 3 && Color.RED.ordinal() < 2; // expect
        var xs = List.of("a", "b");
        r |= xs.size() > 3 && xs.size() < 2; // expect
        r |= FIXED.length() > 3 && FIXED.length() < 2; // expect
        r |= this.fixed.length() > 3 && this.fixed.length() < 2; // expect
        r |= ARRAY.length > 3 && ARRAY.length < 2; // expect
        return r;
    }

    boolean alwaysTrue(int x, double d) {
        boolean r = x != 1 || x != 2; // expect
        r &= x >= 0 || x < 0; // expect
        r &= x < 10 || x > 5; // expect
        r &= d != 1 || d != 2; // expect
        return r;
    }

    boolean fine(int x, int y, double d, List<String> list, java.util.Iterator<Integer> it, java.io.InputStream in, StringBuilder sb) throws Exception {
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
        // a list may change between two calls (another thread, a concurrent
        // list), and so may a StringBuilder
        r |= list.size() > 3 && list.size() < 2;
        r |= sb.length() > 3 && sb.length() < 2;
        // another thread may replace the string or the array between the reads
        r |= text.length() > 3 && text.length() < 2;
        // or a field read twice
        r |= value > 3 && value < 2;
        r |= this.state == 1 && this.state == 2;
        r |= array.length > 3 && array.length < 2;
        // a list assigned again, or a mutable one
        var ys = List.of("a");
        ys = new java.util.ArrayList<>(ys);
        r |= ys.size() > 3 && ys.size() < 2;
        // a size() of the project's own may answer differently each time
        r |= size() > 3 && size() < 3;
        return r;
    }

    int size() {
        int result = state;
        state -= 2;
        return result;
    }

    boolean advance() {
        state++;
        return true;
    }
}
