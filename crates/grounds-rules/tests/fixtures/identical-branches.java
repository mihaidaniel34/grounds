class A {
    int m(boolean c, int a) {
        if (c) { // expect
            a++;
        } else {
            a++;
        }
        int b = c ? a : a; // expect
        if (a == 1) {
            System.out.println("one");
            a = a * 2;
        } else if (a == 2) { // expect
            System.out.println("one");
            a = a * 2;
        } else if (a == 3) {
            System.out.println("three");
        }
        switch (a) { // expect
            case 1: a++; break;
            default: a++; break;
        }
        return c ? a : b;
    }
}
class Chains {
    String nonAdjacent(char c, char open, boolean quoted, StringBuilder out) {
        // the middle test must run first: the duplicates are not adjacent
        if (quoted) {
            out.append(c);
            out.append('!');
        } else if (c == open) {
            out.append('(');
        } else {
            out.append(c);
            out.append('!');
        }
        return out.toString();
    }
    void patterns(Object o, StringBuilder out) {
        // each condition binds its own pattern variable: cannot be joined with ||
        if (o instanceof java.util.OptionalInt oi && oi.isEmpty()) {
            out.append("none");
            out.append('.');
        } else if (o instanceof java.util.OptionalLong ol && ol.isEmpty()) {
            out.append("none");
            out.append('.');
        }
    }
    String typeTable(Object type) {
        return switch (type) {
            case Integer i -> String.valueOf(java.util.Map.of("type", "integer", "format", "int32"));
            case Long l -> String.valueOf(java.util.Map.of("type", "integer", "format", "int32"));
            default -> "object";
        };
    }
    void adjacent(int a, StringBuilder out) {
        if (a == 1) {
            out.append("same");
            out.append('.');
        } else if (a == 2) { // expect
            out.append("same");
            out.append('.');
        }
    }
}
class PatternSwitchWithDefault {
    String m(Object o) {
        // the pattern cases differ from the defaults: not "all cases the same"
        return switch (o) {
            case Integer i when i > 0 -> "positive " + i + " of " + String.valueOf(o).length();
            case String s -> "text";
            case null -> "other";
            default -> "other";
        };
    }
}
class PatternUnused {
    // the bound variable is not read: the condition has no effect
    int size(Object o) {
        if (o instanceof String s) { // expect
            return 1;
        } else {
            return 1;
        }
    }
}
class PatternShadow {
    String text = "fallback";
    // the branches read different variables named `text`: not identical
    int length(Object o) {
        if (o instanceof String text) {
            return text.length();
        } else {
            return text.length();
        }
    }
}
