import org.springframework.beans.factory.annotation.Autowired;
class A {
    @Autowired private final Object dep = null; // expect
    @Autowired private Object ok;
}
