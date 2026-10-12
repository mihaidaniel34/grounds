package app;
import org.springframework.context.annotation.*;
import org.springframework.boot.context.properties.*;
import org.springframework.beans.factory.annotation.Qualifier;
// registered by @EnableConfigurationProperties as `<prefix>-<class name>`
@Configuration
@EnableConfigurationProperties(App.Props.class)
class App {
    @ConfigurationProperties("demo")
    public record Props(String host) {}

    @Bean Object consumer(@Qualifier("demo-app.App$Props") Props props) { return new Object(); }
    @Bean Object wrong(@Qualifier("demo") Props props) { return new Object(); } // expect
}

// the project is an application: its context is complete
@org.springframework.boot.autoconfigure.SpringBootApplication class FixtureApp {}
