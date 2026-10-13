// grounds: no-findings (a library: no @SpringBootApplication or @ComponentScan, the consuming application wires its beans)
package app;
import org.springframework.stereotype.Service;
interface Store {}
interface Mailer {}
@Service class DbStore implements Store {}
@Service class Uses {
    Uses(Store s, Mailer m) {} 
}
@Service class Optional1 {
    Optional1(java.util.Optional<Mailer> m, java.util.List<Mailer> all) {}
}

