// grounds: no-findings
import org.springframework.web.bind.annotation.*;
@RestController
class A {
    @GetMapping("/items")
    String items(@PathVariable(name = "id", required = false) Long id) { return id == null ? "all" : id.toString(); }
    @GetMapping("/things")
    String things(@PathVariable("id") java.util.Optional<Long> id) { return ""; }
}
