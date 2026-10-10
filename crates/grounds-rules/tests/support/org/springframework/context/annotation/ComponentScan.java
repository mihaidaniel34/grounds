package org.springframework.context.annotation; public @interface ComponentScan { String[] value() default {}; String[] basePackages() default {}; }
