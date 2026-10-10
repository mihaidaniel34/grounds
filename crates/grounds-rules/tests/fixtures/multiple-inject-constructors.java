import org.springframework.beans.factory.annotation.Autowired;
class A {
    @Autowired A(String a) {}
    @Autowired A(Integer b) {} // expect
}
class B {
    @Autowired B(String a) {}
    @Autowired(required = false) B(Integer b) {}
}
