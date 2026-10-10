import org.springframework.stereotype.Service;
@Service abstract class Base {} // expect
@Service interface Api {} // expect
@Service class Concrete {}
