// grounds: no-findings
import org.springframework.web.bind.annotation.*;
@RestController
class A {
    @PutMapping(Urls.IMAGE_URL + "/info")
    String update(@PathVariable String type, @PathVariable String key) { return ""; }
}
