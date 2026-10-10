package org.springframework.web.bind.annotation; public @interface CrossOrigin { String[] value() default {}; String[] origins() default {}; }
