import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.stereotype.Service;
class Helper {
    @Autowired private Object dep; // expect
}
@Service class Real {
    @Autowired private Object dep;
}
class ViaBeanMethod {
    @Autowired private Object dep;
}
@org.springframework.context.annotation.Configuration class Cfg {
    @org.springframework.context.annotation.Bean ViaBeanMethod v() { return new ViaBeanMethod(); }
}
interface UserService {}
class LdapUserService implements UserService {
    @Autowired private Object template;
}
@org.springframework.context.annotation.Configuration class AuthCfg {
    @org.springframework.context.annotation.Bean UserService users() { return new LdapUserService(); }
}
interface Unrelated {}
class NotCreated implements Unrelated {
    @Autowired private Object dep; // expect
}
class GreetingFunction {
    @jakarta.inject.Inject Object service;
    @io.quarkus.funqy.Funq public String greet(String name) { return name; }
}
