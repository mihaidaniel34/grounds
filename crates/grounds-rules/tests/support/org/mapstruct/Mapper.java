package org.mapstruct;

import java.lang.annotation.*;

@Retention(RetentionPolicy.CLASS)
@Target(ElementType.TYPE)
public @interface Mapper {
    String componentModel() default "default";
    Class<?>[] uses() default {};
}
