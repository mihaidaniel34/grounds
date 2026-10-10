import org.springframework.stereotype.Service;
import org.springframework.stereotype.Component;
import jakarta.inject.Singleton;
@Service abstract class Base {} // expect
@Service interface Api {} // expect
@Service class Concrete {}
@Component abstract class WithLookup { @org.springframework.beans.factory.annotation.Lookup abstract Object create(); }
@Singleton interface MicronautScoped {}
@Singleton abstract class MicronautIntroduction { abstract String test(String name); }
