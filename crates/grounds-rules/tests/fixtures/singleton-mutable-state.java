import org.springframework.stereotype.Service;
@Service class Counter {
    private int count; // expect
    private final java.util.Map<String, String> cache = new java.util.HashMap<>();
    private String config;
    Counter() { config = "x"; }
    void inc() { count++; }
}
