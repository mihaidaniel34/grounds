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
