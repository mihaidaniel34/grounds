package org.springframework.scheduling.annotation; public @interface Scheduled { String cron() default ""; long fixedRate() default -1; }
