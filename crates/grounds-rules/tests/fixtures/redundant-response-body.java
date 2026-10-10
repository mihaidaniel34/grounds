import org.springframework.web.bind.annotation.*;
@RestController class A {
    @ResponseBody @GetMapping("/x") String x() { return ""; } // expect
}
@org.springframework.stereotype.Controller class B {
    @ResponseBody @GetMapping("/y") String y() { return ""; }
}
