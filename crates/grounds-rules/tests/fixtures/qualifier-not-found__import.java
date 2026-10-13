package app;
import org.springframework.context.annotation.*;
import org.springframework.stereotype.Component;
import org.springframework.boot.context.properties.ConfigurationProperties;
import org.springframework.beans.factory.annotation.Qualifier;
import java.lang.annotation.*;
// classes registered by @Import are named by their class name, even inside
// the scanned packages; composed stereotypes name properties classes
@Configuration @ComponentScan("app") @Import(Plain.class)
class App {
    public static void main(String[] args) {}
    @Bean Object imported(@Qualifier("app.Plain") Plain p) { return new Object(); }
    @Bean Object composed(@Qualifier("props") Props p) { return new Object(); }
    @Bean Object wrong(@Qualifier("plain") Plain p) { return new Object(); } // expect
}
class Plain {}
@Target(ElementType.TYPE) @Retention(RetentionPolicy.RUNTIME) @Component
@interface MyComponent {}
@MyComponent @ConfigurationProperties("demo")
class Props {}
