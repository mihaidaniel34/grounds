import org.springframework.context.annotation.*;
import org.springframework.stereotype.Component;
@Component class Lite {
    @Bean Object a() { return new Object(); }
    @Bean Object b() { return a(); } // expect
}
@Configuration class Full {
    @Bean Object a() { return new Object(); }
    @Bean Object b() { return a(); }
}
@Configuration(proxyBeanMethods = false) class NoProxy {
    @Bean Object a() { return new Object(); }
    @Bean Object b() { return a(); } // expect
}
