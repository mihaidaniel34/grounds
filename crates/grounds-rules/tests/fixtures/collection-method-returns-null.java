import java.util.*;
class A {
    List<String> m(int x) {
        if (x > 0) return new ArrayList<>();
        return null; // expect
    }
    String[] arr() { return null; } // expect
    Map<String, String> map() { return null; } // expect
    @javax.annotation.Nullable List<String> ok() { return null; }
    String s() { return null; }
}
class NullableArrays {
    static byte @org.jspecify.annotations.Nullable [] typePosition(int n) { if (n < 0) return null; return new byte[n]; }
    static void fail(RuntimeException e) { throw e; }
    static java.util.Map<String, Object> parse(String s) {
        try { return new java.util.HashMap<>(); } catch (RuntimeException e) { fail(e); return null; }
    }
}
