package org.springframework.stereotype; @Component public @interface Repository { String value() default ""; }
