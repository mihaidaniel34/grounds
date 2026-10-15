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
// constants read only by annotation arguments are used
@interface Tag { String value() default ""; String[] names() default {}; Tag[] nested() default {}; int size() default 0; }
@Tag(OnAnnotations.ON_CLASS)
class OnAnnotations {
    private static final String ON_CLASS = "c";
    private static final String ON_METHOD = "m";
    private static final String IN_ARRAY = "a";
    private static final String NESTED = "n";
    private static final String ON_FIELD = "f";
    private static final String ON_PARAM = "p";
    private static final int SIZE = 2;
    private static final String UNREAD = "u"; // expect
    @Tag(ON_FIELD) private int tagged;
    @Tag(value = ON_METHOD, size = SIZE + 1)
    void a() {}
    @Tag(names = {"x", IN_ARRAY})
    void b() {}
    @Tag(nested = @Tag(NESTED))
    void c(@Tag(ON_PARAM) int p) {}
}
