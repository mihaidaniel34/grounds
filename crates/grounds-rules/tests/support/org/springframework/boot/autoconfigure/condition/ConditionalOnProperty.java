package org.springframework.boot.autoconfigure.condition; public @interface ConditionalOnProperty { String[] name() default {}; String havingValue() default ""; }
