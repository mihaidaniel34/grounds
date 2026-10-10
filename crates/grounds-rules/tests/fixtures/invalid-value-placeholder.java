import org.springframework.beans.factory.annotation.Value;
class A {
    @Value("${server.port") int port; // expect
    @Value("$server.host") String host; // expect
    @Value("${server.name:default}") String name;
    @Value("#{systemProperties['x']}") String spel;
}
