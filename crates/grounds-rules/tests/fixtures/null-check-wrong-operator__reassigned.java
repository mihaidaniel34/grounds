// grounds: no-findings
class A {
    static boolean ready(String value) {
        return value != null || ((value = "fallback") != null && !value.isEmpty());
    }
}
