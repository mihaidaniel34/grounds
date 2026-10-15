package org.springframework.test.web.servlet.result;
public abstract class MockMvcResultMatchers {
  public static StatusResultMatchers status() { return new StatusResultMatchers(); }
}
