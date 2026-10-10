package app;
import org.springframework.stereotype.Service;
import org.springframework.boot.context.properties.ConfigurationProperties;
// registered by @EnableConfigurationProperties / @ConfigurationPropertiesScan
@ConfigurationProperties("mail")
class MailProperties { String host; }
interface Mailer {}
@Service class Uses {
    Uses(MailProperties props,
         Mailer mailer) {} // expect
}
