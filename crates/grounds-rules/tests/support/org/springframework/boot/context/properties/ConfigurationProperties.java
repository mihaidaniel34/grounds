package org.springframework.boot.context.properties; public @interface ConfigurationProperties { String value() default ""; String prefix() default ""; }
