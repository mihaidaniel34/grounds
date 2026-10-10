package org.springframework.stereotype; @Component public @interface Service { String value() default ""; }
