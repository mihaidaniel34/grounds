import org.springframework.web.bind.annotation.*;
import org.springframework.web.servlet.config.annotation.CorsRegistration;
@CrossOrigin // expect
class A {
    @CrossOrigin(origins = "*") // expect
    void m(CorsRegistration r) {
        r.allowedOrigins("*"); // expect
        r.allowedOrigins("https://example.com");
    }
    @CrossOrigin(origins = "https://app.example.com")
    void ok() {}
}
