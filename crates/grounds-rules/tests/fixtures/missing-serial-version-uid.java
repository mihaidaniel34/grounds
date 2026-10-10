import java.io.Serializable;
class A implements Serializable { } // expect
class B implements Serializable { private static final long serialVersionUID = 1L; }
class E extends Exception { }
