package org.junit; public @interface Test { Class<? extends Throwable> expected() default None.class; class None extends Throwable {} }
