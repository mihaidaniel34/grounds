import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.context.annotation.AnnotationConfigApplicationContext;
class Sample {
    @Autowired Object value;
    static void main(String[] a) {
        AnnotationConfigApplicationContext ctx = new AnnotationConfigApplicationContext();
        ctx.registerBean(Sample.class);
    }
}
class NotABean {
    @Autowired Object value; // expect
}
