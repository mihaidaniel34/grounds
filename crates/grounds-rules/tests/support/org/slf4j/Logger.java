package org.slf4j;
public interface Logger {
  void trace(String msg); void trace(String fmt, Object arg); void trace(String fmt, Object a, Object b); void trace(String fmt, Object... args); void trace(String msg, Throwable t);
  void debug(String msg); void debug(String fmt, Object arg); void debug(String fmt, Object a, Object b); void debug(String fmt, Object... args); void debug(String msg, Throwable t);
  void info(String msg); void info(String fmt, Object arg); void info(String fmt, Object a, Object b); void info(String fmt, Object... args); void info(String msg, Throwable t);
  void warn(String msg); void warn(String fmt, Object arg); void warn(String fmt, Object a, Object b); void warn(String fmt, Object... args); void warn(String msg, Throwable t);
  void error(String msg); void error(String fmt, Object arg); void error(String fmt, Object a, Object b); void error(String fmt, Object... args); void error(String msg, Throwable t);
  boolean isDebugEnabled(); boolean isTraceEnabled(); boolean isInfoEnabled();
}
