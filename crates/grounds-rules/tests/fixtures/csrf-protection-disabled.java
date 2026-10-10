import org.springframework.security.config.annotation.web.builders.HttpSecurity;
class A {
    void m(HttpSecurity http) {
        http.csrf().disable(); // expect
        http.csrf(c -> c.disable()); // expect
    }
}
