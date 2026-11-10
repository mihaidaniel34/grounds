class A {
    private static int count;
    private static final Object LOCK = new Object();
    void inc() {
        count++; // expect
        count = 5; // expect
    }
    static void reset() { count = 0; }
    synchronized void safe() { count = 1; }
    A() { }
}
class ContextHolder implements org.springframework.context.ApplicationContextAware {
    private static org.springframework.context.ApplicationContext context;
    private static String secret;
    private static int requests;
    // container callbacks that run once in a singleton's life
    public void setApplicationContext(org.springframework.context.ApplicationContext ctx) { context = ctx; }
    @jakarta.annotation.PostConstruct
    void init() { secret = "x"; }
    @org.springframework.beans.factory.annotation.Value("${app.secret}")
    void setSecret(String s) { secret = s; }
    // still reported: runs on every call
    void handle() { requests++; } // expect
    void setContext(org.springframework.context.ApplicationContext ctx) { context = ctx; } // expect
    // the name of another Aware callback, but neither its interface nor its signature
    public void setEnvironment(String env) { secret = env; } // expect
    // a startup observer runs once; an application-event observer runs per event
    void onStart(@jakarta.enterprise.event.Observes io.quarkus.runtime.StartupEvent e) { secret = "y"; }
    void onOrder(@jakarta.enterprise.event.Observes String order) { requests++; } // expect
}
class OwnContextType implements org.springframework.context.ApplicationContextAware {
    static class ApplicationContext { }
    private static Object ctx;
    public void setApplicationContext(org.springframework.context.ApplicationContext c) { ctx = c; }
    // an overload taking a project type of the same simple name is no callback
    public void setApplicationContext(ApplicationContext c) { ctx = c; } // expect
}
interface InitializingBean { }
class NotSpringsInterface implements InitializingBean {
    private static int started;
    // a project interface with Spring's simple name
    public void afterPropertiesSet() { started++; } // expect
}
