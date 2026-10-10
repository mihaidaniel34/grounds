import org.springframework.web.bind.annotation.*;
@RestController @RequestMapping("/api")
class A {
    @RequestMapping("/a") String a() { return ""; } // expect
    @RequestMapping(value = "/b", method = RequestMethod.GET) String b() { return ""; }
    @GetMapping("/c") String c() { return ""; }
}
