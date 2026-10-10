// Java 22: `_` inside a record pattern cannot be rewritten for the parser
class Test {
    record R(int x) {}
    void run() {}
    int f(Object x) {
        return switch (x) {
            case R(_) -> 1;
            case String _, Integer _ -> 2;
            default -> 0;
        };
    }
    void g() { this.walk(); } // expect
}
