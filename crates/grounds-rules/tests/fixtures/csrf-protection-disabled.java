import org.springframework.security.config.annotation.web.builders.HttpSecurity;
class A {
    void m(HttpSecurity http) {
        http.csrf().disable(); // expect
        http.csrf(c -> c.disable()); // expect
    }
}
// token-authenticated API: no session cookie for a forged request to carry
class Stateless {
    enum SessionCreationPolicy { STATELESS, IF_REQUIRED }
    void lambda(HttpSecurity http) {
        http.csrf(c -> c.disable())
            .sessionManagement(s -> s.sessionCreationPolicy(SessionCreationPolicy.STATELESS));
    }
    void chained(HttpSecurity http) {
        http.csrf().disable();
        http.sessionManagement().sessionCreationPolicy(SessionCreationPolicy.STATELESS);
    }
    void session(HttpSecurity http) {
        http.csrf().disable(); // expect
        http.sessionManagement(s -> s.sessionCreationPolicy(SessionCreationPolicy.IF_REQUIRED));
    }
}
