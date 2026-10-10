package org.springframework.stereotype; @Component public @interface Controller { String value() default ""; }
