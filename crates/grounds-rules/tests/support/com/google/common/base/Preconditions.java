package com.google.common.base;
public final class Preconditions {
    public static void checkArgument(boolean expression, Object errorMessage) { if (!expression) throw new IllegalArgumentException(String.valueOf(errorMessage)); }
    public static void checkState(boolean expression, Object errorMessage) { if (!expression) throw new IllegalStateException(String.valueOf(errorMessage)); }
    public static <T> T checkNotNull(T reference) { if (reference == null) throw new NullPointerException(); return reference; }
}
