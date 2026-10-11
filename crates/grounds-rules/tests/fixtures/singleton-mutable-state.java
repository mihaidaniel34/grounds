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
