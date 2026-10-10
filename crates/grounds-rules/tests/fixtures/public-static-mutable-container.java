import java.util.*;
class A {
    public static final String[] NAMES = {"a", "b"}; // expect
    public static final List<String> ITEMS = new ArrayList<>(); // expect
    public static final List<String> OK = List.of("x");
    public static final String[] EMPTY = new String[0];
    private static final int[] HIDDEN = {1};
}
