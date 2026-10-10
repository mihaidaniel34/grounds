import java.io.Serializable;
class A implements Serializable {
    private final long serialVersionUID = 1L; // expect
}
class B implements Serializable {
    static final int serialVersionUID = 1; // expect
}
class C implements Serializable {
    private static final long serialVersionUID = 1L;
}
