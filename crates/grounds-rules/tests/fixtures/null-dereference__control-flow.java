import java.io.*;
import java.util.function.Supplier;
@interface Nullable {}
class FlowCases {
    static void fail() throws IOException { throw new IOException(); }

    // the handler sees the state before fail(), not after `x = null`
    static void exceptionPoint() {
        Object x = new Object();
        try {
            fail();
            x = null;
        } catch (IOException ex) {
            x.toString();
        }
    }

    // break and continue run the finally block
    static void breakFinally() {
        Object x = null;
        while (true) {
            try { break; }
            finally { x = new Object(); }
        }
        x.toString();
    }
    static void continueFinally() {
        Object x = null;
        for (int i = 0; i < 1; i++) {
            try { continue; }
            finally { x = new Object(); }
        }
        x.toString();
    }

    // only java.util.Objects.isNull proves null
    static boolean isNull(Object o) { return true; }
    static void customPredicate() {
        Object x = new Object();
        if (isNull(x)) { x.toString(); }
    }
    static void objectsIsNull(Object x) {
        if (java.util.Objects.isNull(x)) { x.toString(); } // expect
    }

    // static members through a null reference do not dereference it
    static String staticViaNull() {
        String s = null;
        return s.valueOf(42);
    }

    // int arithmetic wraps: the branch is never taken
    static void overflow() {
        Object x = new Object();
        if (2147483647 + 1 > 0) x = null;
        x.toString();
    }
    static void shift() {
        Object x = new Object();
        if ((1 << 32) != 1) x = null;
        x.toString();
    }

    // javac picks the Supplier overload (the lambda returns a value)
    @Nullable static String choose(Runnable action) { return null; }
    static String choose(Supplier<String> action) { return action.get(); }
    static int lambdaOverload() {
        String result = choose(() -> "ok");
        return result.length();
    }

    // constants take their expression's type: n + 1 is long arithmetic
    static void longLocal() {
        final long n = 2147483647;
        Object x = new Object();
        if (n + 1 < 0) x = null;
        x.toString();
    }
    // long vs float compares as float in Java: not folded
    static void floatPromotion() {
        Object x = new Object();
        if (16777217L != 16777216f) x = null;
        x.toString();
    }

    // a return inside a local type does not make the lambda value-returning
    @Nullable static String pick(Supplier<String> s) { return null; }
    static String pick(Runnable r) { return "ok"; }
    static int localTypeInLambda() {
        String s = pick(() -> {
            enum E { A; int n() { return 1; } }
        });
        return s.length();
    }

    // still reported
    static void stillNull() {
        Object x = null;
        x.toString(); // expect
    }
}
