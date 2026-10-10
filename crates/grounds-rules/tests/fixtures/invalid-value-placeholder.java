import org.springframework.beans.factory.annotation.Value;
class A {
    @Value("${server.port") int port; // expect
    @Value("$server.host") String host; // expect
    @Value("${server.name:default}") String name;
    @Value("#{systemProperties['x']}") String spel;
    static final String KEY = "server.timeout";
    @Value("${" + KEY + "}") int timeout;
    @Value("${" + KEY + ":30}") int timeoutDefault;
    @Value("${" + KEY) int broken; // expect
    @Value("${" + Unknown.KEY + "}") int unknown;
}
