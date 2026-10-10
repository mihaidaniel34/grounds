class A {
    private int unused; // expect
    private int writeOnly; // expect
    private int used;
    private static final long serialVersionUID = 1L;
    @SuppressWarnings("x") private int annotated;
    void m() {
        writeOnly = 5;
        System.out.println(used);
    }
}
@lombok.Getter
class B { private int viaLombok; }
// builder fields read by the enclosing class: `builder.count` must not
// resolve to the outer `count`
class Usage {
    private final Integer count;
    private Usage(Builder builder) { this.count = builder.count; }
    Integer count() { return count; }
    static class Builder {
        private Integer count;
        private Integer dropped; // expect
        Builder count(Integer count) { this.count = count; return this; }
        Builder dropped(Integer dropped) { this.dropped = dropped; return this; }
        Usage build() { return new Usage(this); }
    }
}
