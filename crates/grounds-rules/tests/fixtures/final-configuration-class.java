import org.springframework.context.annotation.*;
@Configuration final class A { } // expect
@Configuration class B {
    @Bean final Object x() { return null; } // expect
    @Bean private Object y() { return null; } // expect
    @Bean Object ok() { return null; }
}
@Configuration(proxyBeanMethods = false) final class C { @Bean final Object z() { return null; } }
