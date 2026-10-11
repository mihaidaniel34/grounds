package org.springframework.util;
public abstract class Assert {
    public static void state(boolean expression, String message) { if (!expression) throw new IllegalStateException(message); }
    public static void isTrue(boolean expression, String message) { if (!expression) throw new IllegalArgumentException(message); }
    public static void notNull(Object object, String message) { if (object == null) throw new IllegalArgumentException(message); }
}
