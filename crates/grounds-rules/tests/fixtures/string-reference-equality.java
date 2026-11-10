class A {
    static final String K = "k";
    boolean m(String a, String b, Object o) {
        if (a == b) return true;
        if (a != "x") return false; // expect
        if (a == null) return false;
        if (o == a) return true;
        if (K == "k") return true;
        return a.equals(b);
    }
}
class FastPaths {
    final String name;
    FastPaths(String name) { this.name = name; }
    boolean either(String a, String b) { return a == b || a.equals(b); }
    boolean nullSafe(String a, String b) { return a == b || (a != null && a.equals(b)); }
    boolean differs(String a, String b) { return a != b && !java.util.Objects.equals(a, b); }
    int compare(String a, String b) {
        if (a == b) {
            return 0;
        }
        if (a == null) return -1;
        if (b == null) return 1;
        return a.compareTo(b);
    }
    FastPaths withName(String name) { return this.name == name ? this : new FastPaths(name); }
    FastPaths withName2(String name) {
        if (this.name == name) return this;
        return new FastPaths(name);
    }
    // the result decides behaviour and nothing compares contents
    boolean other(String a, String b, String c) { return a == b || a.equals(c); } // expect
    int rank(String a, String b) {
        if (a == b) return 0; // expect
        return a.length() - b.length();
    }
    String pick(String name) { return this.name == name ? "same" : "changed"; } // expect
    FastPaths keep(String name) { return this.name != name ? this : new FastPaths(name); } // expect
    boolean converged(String next, String result) {
        if (next == result) return true; // expect
        return false;
    }
}
