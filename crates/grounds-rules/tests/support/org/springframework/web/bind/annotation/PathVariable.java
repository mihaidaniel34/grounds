package org.springframework.web.bind.annotation; public @interface PathVariable { String value() default ""; String name() default ""; boolean required() default true; }
