class Suppressions {
    boolean plain(String a, String b) {
        return a == b; // expect
    }
    @SuppressWarnings("string-reference-equality")
    boolean byId(String a, String b) { return a == b; }
    @SuppressWarnings("java:S4973")
    boolean bySonarId(String a, String b) { return a == b; }
    @SuppressWarnings({"unchecked", "ES_COMPARING_STRINGS_WITH_EQ"})
    boolean bySpotbugsId(String a, String b) { return a == b; }
    @SuppressWarnings("unchecked")
    boolean unrelated(String a, String b) { return a == b; } // expect
    boolean nosonar(String a, String b) { return a == b; } // NOSONAR
    boolean ignoreAll(String a, String b) { return a == b; } // grounds-ignore
    boolean ignoreThis(String a, String b) { return a == b; } // grounds-ignore: string-reference-equality
    boolean ignoreOther(String a, String b) { return a == b; } // grounds-ignore: dead-store  // expect
    boolean previousLine(String a, String b) {
        // grounds-ignore
        return a == b;
    }
}
@SuppressWarnings("string-reference-equality")
class WholeClass {
    boolean m(String a, String b) { return a == b; }
}
