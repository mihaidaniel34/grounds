// grounds: no-findings
class A {
    static boolean ready(String value) {
        return value != null || ((value = "fallback") != null && !value.isEmpty());
    }
    static boolean guarded(String s, boolean required) {
        return s != null || (!required || (s != null && s.isEmpty()));
    }
}
