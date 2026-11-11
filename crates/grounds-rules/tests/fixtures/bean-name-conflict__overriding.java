// grounds: bean-overriding
package app;
import org.springframework.stereotype.Service;
import org.springframework.context.annotation.*;
@Service("store") class DbStore {} // expect
@Service("store") class MemStore {} // expect
@Service class Mailer {}
@Configuration class Cfg {
    @Bean(name = "mailer") Object other() { return null; }
}

// the project is an application, started by its main: its context is complete
@org.springframework.boot.autoconfigure.SpringBootApplication class FixtureApp { public static void main(String[] args) {} }
