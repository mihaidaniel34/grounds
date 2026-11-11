import java.util.*;
class A {
    public static final String[] NAMES = {"a", "b"}; // expect
    public static final List<String> ITEMS = new ArrayList<>(); // expect
    public static final List<String> OK = List.of("x");
    public static final String[] EMPTY = new String[0];
    private static final int[] HIDDEN = {1};
}
class EmptyArrays {
    public static final String[] NONE = new String[] {};
    public static final String[] NONE2 = new String[]{ };
}
class OuterDimension {
    public static final int[][] EMPTY_ROWS = new int[0][3];
    public static final int[][] ROWS = new int[3][0]; // expect
}
