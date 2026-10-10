package org.springframework.web.bind.annotation; import org.springframework.stereotype.Controller; @Controller @ResponseBody public @interface RestController { String value() default ""; }
