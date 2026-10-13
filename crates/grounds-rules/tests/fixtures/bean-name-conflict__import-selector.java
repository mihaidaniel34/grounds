package app;
import org.springframework.context.annotation.*;
import org.springframework.core.type.AnnotationMetadata;
// an imported ImportSelector is processed, not registered as a bean
@Configuration @ComponentScan("app") @Import(App.Selector.class)
class App {
    public static void main(String[] args) {}
    public static class Selector implements ImportSelector {
        public String[] selectImports(AnnotationMetadata m) { return new String[0]; }
    }
    @Bean Selector selector() { return new Selector(); }
}
@Configuration class One { @Bean Object dup() { return null; } } // expect
@Configuration class Two { @Bean Object dup() { return null; } } // expect
