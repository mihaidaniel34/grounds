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
