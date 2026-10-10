import org.springframework.web.bind.annotation.*;
@interface CustomPostMapping { String path(); }
@RestController @RequestMapping("/users/{tenant}")
class A {
    @GetMapping("/{id}")
    String get(@PathVariable String tenant, @PathVariable("id") long id, @PathVariable String name) { return ""; } // expect
    @GetMapping("/{userId}/posts")
    String posts(@PathVariable("postId") long p) { return ""; } // expect
    @GetMapping("/{id}")
    String ok(@PathVariable long id) { return ""; }
    @CustomPostMapping(path = "/{key}/search")
    String custom(@PathVariable long key) { return ""; }
}
