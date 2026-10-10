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
class Logged {
    static final java.util.logging.Logger LOG = java.util.logging.Logger.getLogger(Logged.class.getName());
    @Autowired Object value; // expect
}
