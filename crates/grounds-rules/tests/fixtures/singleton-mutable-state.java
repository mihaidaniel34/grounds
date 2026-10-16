import org.springframework.stereotype.Service;
@Service class Counter {
    private int count; // expect
    private final java.util.Map<String, String> cache = new java.util.HashMap<>();
    private String config;
    Counter() { config = "x"; }
    void inc() { count++; }
}
@org.springframework.stereotype.Service
class ConfiguredService {
    private String endpoint;
    @org.springframework.beans.factory.annotation.Value("${endpoint}")
    public void setEndpoint(String endpoint) { this.endpoint = endpoint; }
    String endpoint() { return endpoint; }
}
@org.springframework.stereotype.Component
class Coordinator implements org.springframework.beans.factory.InitializingBean {
    private Thread worker;
    private String format;
    private Object pool;
    private java.util.concurrent.atomic.AtomicBoolean scan;
    private int requests; // expect
    Coordinator() { initialize(); }
    private void initialize() { scan = new java.util.concurrent.atomic.AtomicBoolean(true); }
    public synchronized void start() { worker = new Thread(() -> { }); }
    public void afterPropertiesSet() { format = "x"; }
    @org.springframework.context.event.EventListener(org.springframework.boot.context.event.ApplicationReadyEvent.class)
    public void onReady() { pool = new Object(); }
    public void handle() { requests++; }
}
