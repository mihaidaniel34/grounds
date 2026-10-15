class A {
    String m(int i, Object o) {
        String a = "n=" + String.valueOf(i); // expect
        String b = String.valueOf(i);
        String c = String.valueOf(i) + 1;
        return String.valueOf(o) + "x"; // expect
    }

    // removing both calls would turn "12" into 3: only the second one is redundant
    String twoInts(int a, int b) {
        return String.valueOf(a) + String.valueOf(b); // expect
    }

    String twoIntsSplit(int a, int b) {
        return String.valueOf(a)
            + String.valueOf(b); // expect
    }

    String separated(int a, int b) {
        return String.valueOf(a) // expect
            + ":"
            + String.valueOf(b); // expect
    }

    String keptLeft(int a, int b) {
        return String.valueOf(a) + b;
    }

    String charArray(char[] cs) {
        return "x" + String.valueOf(cs);
    }

    // a + b is numeric addition before the call is reached
    String numericPrefix(int a, int b, int c) {
        return a + b + String.valueOf(c);
    }

    String numericPrefixThenString(int a, int b, int c) {
        return a + b + "" + String.valueOf(c); // expect
    }

    String chars(char x, char y) {
        String s = x + String.valueOf(y);
        String t = String.valueOf(x) + y;
        return "" + String.valueOf(x); // expect
    }

    String threeInts(int a, int b, int c) {
        return String.valueOf(a)
            + String.valueOf(b) // expect
            + String.valueOf(c); // expect
    }

    String trailingString(int a, int b) {
        return String.valueOf(a)
            + String.valueOf(b) // expect
            + "x";
    }

    // the parenthesized sum must stay a String on its own
    String nestedRight(int a, int b) {
        return "x" + (String.valueOf(a)
            + String.valueOf(b)); // expect
    }

    String nestedLeft(int a, int b) {
        return (String.valueOf(a)
            + String.valueOf(b)) // expect
            + "x";
    }

    String nestedString(int a, int b) {
        return String.valueOf(a) // expect
            + (String.valueOf(b) + "y"); // expect
    }

    String numericGroup(int a, int b) {
        return String.valueOf(a) + (b + 1);
    }

    String stringArg(String s, int a) {
        return a + String.valueOf(s); // expect
    }

    String sumArg(int a, int b) {
        return "x" + String.valueOf(a + b); // expect
    }
}
