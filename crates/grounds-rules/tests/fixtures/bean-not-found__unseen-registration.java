package unseen;
import org.springframework.stereotype.Service;
import com.acme.Registered;

// "no bean" needs evidence: Mailer's only implementation carries nothing
// but Lombok, which registers no bean, and nothing produces it; Notifier's
// implementation carries an annotation the model cannot classify (a
// library scanning for it), so it may well be a bean
interface Mailer {}
interface Notifier {}
@Registered class SmsNotifier implements Notifier {}
@lombok.Data class Draft implements Mailer {}
@Service class Uses {
    Uses(Mailer mailer, // expect
         Notifier notifier) {}
}

@org.springframework.boot.autoconfigure.SpringBootApplication class FixtureApp {}
