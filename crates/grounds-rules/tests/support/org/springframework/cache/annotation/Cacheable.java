package org.springframework.cache.annotation; public @interface Cacheable { String[] value() default {}; }
