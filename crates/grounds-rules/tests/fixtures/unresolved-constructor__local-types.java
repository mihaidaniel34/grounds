// local types with the same name in different methods are distinct
class Tests {
    void first() {
        record TestCase(String a, String b, Class<?> type, Class<? extends Throwable> error) {}
        new TestCase("a", "b", String.class, null);
    }
    void second() {
        record TestCase(String a, String b) {}
        new TestCase("a", "b");
        new TestCase("a"); // expect
    }
}
