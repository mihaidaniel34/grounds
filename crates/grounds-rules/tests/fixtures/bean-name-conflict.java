package app;
import org.springframework.stereotype.Service;
import org.springframework.context.annotation.*;
@Service("store") class DbStore {} // expect
@Service("store") class MemStore {} // expect
@Service class Mailer {} // expect
@Configuration class Cfg {
    @Bean Object clock() { return null; }
    @Bean(name = "mailer") Object other() { return null; } // expect
}
