package io.micronaut.context.annotation;
public @interface Replaces { Class<?> value() default Object.class; Class<?> bean() default Object.class; String named() default ""; }
