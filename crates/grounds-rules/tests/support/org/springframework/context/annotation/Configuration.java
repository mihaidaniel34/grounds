package org.springframework.context.annotation; import org.springframework.stereotype.Component; @Component public @interface Configuration { boolean proxyBeanMethods() default true; }
