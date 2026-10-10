import org.springframework.beans.factory.annotation.*;
class A {
    @Autowired private static Object dep; // expect
    @Value("${x}") private static String v; // expect
    @Autowired private Object ok;
}
