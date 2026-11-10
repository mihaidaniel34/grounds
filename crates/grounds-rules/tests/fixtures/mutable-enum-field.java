enum Level {
    LOW, HIGH;
    private int count; // expect
    private final String label = "x";
    void hit() { count++; }
}
enum Code {
    A(1, "a"), B(2, "b");
    // assigned only by the constructor and never again: effectively immutable
    private int value;
    private String text;
    // still reported: visible to the package, or changed by a method
    int weight; // expect
    private String title; // expect
    Code(int value, String text) { this.value = value; this.text = text; }
    public void setTitle(String title) { this.title = title; }
}
@lombok.Getter @lombok.Setter
enum Settable {
    X;
    private String label; // expect
}
