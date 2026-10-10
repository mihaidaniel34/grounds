package org.springframework.context.annotation; public @interface Bean { String[] value() default {}; String[] name() default {}; }
