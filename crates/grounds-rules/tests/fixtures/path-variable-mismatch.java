import org.springframework.web.bind.annotation.*;
@RestController @RequestMapping("/users/{tenant}")
class A {
    @GetMapping("/{id}")
    String get(@PathVariable String tenant, @PathVariable("id") long id, @PathVariable String name) { return ""; } // expect
    @GetMapping("/{userId}/posts")
    String posts(@PathVariable("postId") long p) { return ""; } // expect
    @GetMapping("/{id}")
    String ok(@PathVariable long id) { return ""; }
}
