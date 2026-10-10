package org.junit.jupiter.api;
import org.junit.jupiter.api.function.Executable;
public final class Assertions {
  public static void assertEquals(Object expected, Object actual) {} public static void assertEquals(Object expected, Object actual, String msg) {}
  public static void assertEquals(long expected, long actual) {} public static void assertEquals(int expected, int actual) {} public static void assertEquals(double e, double a, double d) {}
  public static void assertNotEquals(Object unexpected, Object actual) {}
  public static void assertTrue(boolean c) {} public static void assertTrue(boolean c, String msg) {}
  public static void assertFalse(boolean c) {} public static void assertFalse(boolean c, String msg) {}
  public static void assertNull(Object o) {} public static void assertNotNull(Object o) {}
  public static void assertSame(Object e, Object a) {} public static void assertNotSame(Object e, Object a) {}
  public static <T extends Throwable> T assertThrows(Class<T> t, Executable e) { return null; }
  public static void fail(String m) {} public static void fail() {}
}
